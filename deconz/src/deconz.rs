use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot, watch};

use crate::aps::{Aps, ApsCommand, ApsReader};
use crate::slip;
use crate::{
    ApsDataConfirm, ApsDataRequest, DeviceState, Error, ErrorKind, Platform, Request, Response,
    Result, SequenceId, Version,
};

/// A command from Deconz to the Tx task, representing a serial Request using the Deconz protocol.
struct SerialCommand {
    request: Request,
    sender: oneshot::Sender<Result<Response>>,
}

#[derive(Clone)]
pub struct Deconz {
    commands: mpsc::Sender<SerialCommand>,
    aps_data_requests: mpsc::Sender<ApsCommand>,
}

impl Deconz {
    pub fn new<R, W>(reader: R, writer: W) -> (Self, ApsReader)
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let reader = slip::Reader::new(reader);
        let writer = slip::Writer::new(writer);

        let (commands_tx, commands_rx) = mpsc::channel(1);
        let (device_state_tx, device_state_rx) = watch::channel(DeviceState::default());
        let (aps_data_indications_tx, aps_data_indications_rx) = mpsc::channel(1);
        let (aps_data_requests_tx, aps_data_requests_rx) = mpsc::channel(1);

        let deconz = Self {
            commands: commands_tx,
            aps_data_requests: aps_data_requests_tx,
        };
        let aps_reader = ApsReader {
            rx: aps_data_indications_rx,
        };

        let shared = Arc::new(Shared::default());
        let rx = Rx {
            shared: shared.clone(),
            reader,
            device_state: device_state_tx,
        };
        let tx = Tx {
            shared,
            writer,
            commands: commands_rx,
            sequence_id: 0,
        };

        let aps = Aps {
            deconz: deconz.clone(),
            request_id: 0,
            request_free_slots: false,
            device_state: device_state_rx,
            aps_data_indications: aps_data_indications_tx,
            aps_data_requests: aps_data_requests_rx,
            awaiting: HashMap::new(),
        };

        tokio::spawn(rx.task());
        tokio::spawn(tx.task());
        tokio::spawn(aps.task());

        (deconz, aps_reader)
    }

    pub async fn make_request(&self, request: Request) -> Result<Response> {
        let (sender, receiver) = oneshot::channel();

        self.commands
            .clone()
            .send(SerialCommand { request, sender })
            .await
            .map_err(|_| ErrorKind::ChannelError)?;

        let result = receiver.await.map_err(|_| ErrorKind::ChannelError)?;
        let response = result?;

        Ok(response)
    }

    pub async fn version(&self) -> Result<(Version, Platform)> {
        match self.make_request(Request::Version).await? {
            Response::Version { version, platform } => Ok((version, platform)),
            resp => Err(ErrorKind::UnexpectedResponse(resp.command_id()).into()),
        }
    }

    pub async fn device_state(&self) -> Result<DeviceState> {
        match self.make_request(Request::DeviceState).await? {
            Response::DeviceState(device_state) => Ok(device_state),
            resp => Err(ErrorKind::UnexpectedResponse(resp.command_id()).into()),
        }
    }

    pub async fn aps_data_request(&self, request: ApsDataRequest) -> Result<ApsDataConfirm> {
        let (sender, receiver) = oneshot::channel();

        // Send to Aps task so that it can be sent when the device is ready.
        self.aps_data_requests
            .clone()
            .send(ApsCommand { request, sender })
            .await
            .map_err(|_| ErrorKind::ChannelError)?;

        let result = receiver.await.map_err(|_| ErrorKind::ChannelError)?;
        let aps_data_confirm = result?;

        Ok(aps_data_confirm)
    }
}

/// Shared state between the Rx and Tx tasks. Holds oneshots to send responses to.
#[derive(Default)]
struct Shared {
    awaiting: Mutex<HashMap<SequenceId, oneshot::Sender<Result<Response>>>>,
}

/// Task responsible for receiving responses from adapter over serial using the Deconz protocol.
///
/// Forwards responses to futures awaiting a response using the oneshots registered by Tx task.
/// Broadcasts any update to DeviceState for other tasks (e.g. Aps) to respond to.
struct Rx<R>
where
    R: AsyncRead + Unpin,
{
    shared: Arc<Shared>,
    reader: slip::Reader<R>,
    device_state: watch::Sender<DeviceState>,
}

impl<R> Rx<R>
where
    R: AsyncRead + Unpin,
{
    async fn task(mut self) -> Result<()> {
        loop {
            let frame = match self.read_frame().await {
                Ok(frame) => frame,
                Err(error) => {
                    error!("rx read _frame: {}", error);
                    continue;
                }
            };

            if let Err(error) = self.process_frame(frame).await {
                error!("rx process_frame: {}", error);
            }
        }
    }

    async fn read_frame(&mut self) -> Result<Vec<u8>> {
        let frame = self.reader.read_frame().await?;
        debug!("received = {:?}", frame);

        Ok(frame)
    }

    async fn process_frame(&mut self, frame: Vec<u8>) -> Result<()> {
        let sequence_id = frame[1];

        match Response::from_frame(frame) {
            Ok((sequence_id, response)) => {
                self.broadcast_device_state(&response).await?;
                if response.solicited() {
                    self.route_response(sequence_id, Ok(response)).await?;
                }
            }
            Err(error) => {
                self.route_response(sequence_id, Err(error)).await?;
            }
        }

        Ok(())
    }

    async fn broadcast_device_state(&mut self, response: &Response) -> Result<()> {
        if let Some(device_state) = response.device_state() {
            self.device_state
                .broadcast(device_state)
                .map_err(|_| ErrorKind::ChannelError)?;
        }
        Ok(())
    }

    async fn route_response(
        &mut self,
        sequence_id: SequenceId,
        result: Result<Response>,
    ) -> Result<()> {
        let mut awaiting = self.shared.awaiting.lock().unwrap();

        match awaiting.remove(&sequence_id) {
            Some(sender) => sender.send(result).map_err(|_| ErrorKind::ChannelError)?,
            _ => error!("rx: unexpected response {:?}", result),
        }

        Ok(())
    }
}

/// Task responsible for transmitting requests to adapter over serial using the Deconz protocol.
///
/// Registers oneshot receivers for each request, so that the Rx task can route responses to the
/// correct future.
struct Tx<W>
where
    W: AsyncWrite + Unpin,
{
    shared: Arc<Shared>,
    writer: slip::Writer<W>,
    commands: mpsc::Receiver<SerialCommand>,
    sequence_id: u8,
}

impl<W> Tx<W>
where
    W: AsyncWrite + Unpin,
{
    async fn task(mut self) -> Result<()> {
        while let Some(SerialCommand { request, sender }) = self.commands.recv().await {
            let sequence_id = self.sequence_id();
            let frame = match request.into_frame(sequence_id) {
                Ok(frame) => frame,
                Err(error) => {
                    self.forward_error(sender, error);
                    continue;
                }
            };

            self.register_awaiting(sequence_id, sender);
            if let Err(error) = self.write_frame(frame).await {
                if let Some(sender) = self.deregister_awaiting(sequence_id) {
                    self.forward_error(sender, error);
                }
            }
        }

        Ok(())
    }

    fn forward_error(&self, sender: oneshot::Sender<Result<Response>>, error: Error) {
        if let Err(error) = sender.send(Err(error)) {
            error!("error forwarding error: {:?}", error);
        }
    }

    fn sequence_id(&mut self) -> SequenceId {
        // Increment by 5 each time, as the Deconz stick seems to ignore some requests if the
        // sequence ID matches the sequence ID of an unsolicited frame.
        let old = self.sequence_id;
        self.sequence_id = (self.sequence_id + 5) % 255;
        old
    }

    fn register_awaiting(
        &self,
        sequence_id: SequenceId,
        sender: oneshot::Sender<Result<Response>>,
    ) {
        self.shared
            .awaiting
            .lock()
            .unwrap()
            .insert(sequence_id, sender);
    }

    fn deregister_awaiting(
        &self,
        sequence_id: SequenceId,
    ) -> Option<oneshot::Sender<Result<Response>>> {
        self.shared.awaiting.lock().unwrap().remove(&sequence_id)
    }

    async fn write_frame(&mut self, frame: Vec<u8>) -> Result<()> {
        debug!("sending = {:?}", frame);
        self.writer.write_frame(&frame).await?;
        Ok(())
    }
}
