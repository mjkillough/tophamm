use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot, watch};

use crate::aps::{Aps, ApsCommand, ApsReader};
use crate::slip;
use crate::{
    ApsDataConfirm, ApsDataRequest, DeviceState, ErrorKind, Platform, Request, Response, Result,
    SequenceId, Version,
};

/// A command from Deconz to the Tx task, representing a serial Request using the Deconz protocol.
struct SerialCommand {
    request: Request,
    sender: oneshot::Sender<Response>,
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

        let response = receiver.await.map_err(|_| ErrorKind::ChannelError)?;

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
    awaiting: Mutex<HashMap<SequenceId, oneshot::Sender<Response>>>,
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
            if let Err(error) = self.process_frame().await {
                error!("rx: {}", error);
            }
        }
    }

    async fn process_frame(&mut self) -> Result<()> {
        let frame = self.reader.read_frame().await?;
        debug!("received = {:?}", frame);
        let (sequence_id, response) = Response::from_frame(frame)?;

        self.broadcast_device_state(&response).await?;
        if response.solicited() {
            self.route_response(sequence_id, response).await?;
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

    async fn route_response(&mut self, sequence_id: SequenceId, response: Response) -> Result<()> {
        let mut awaiting = self.shared.awaiting.lock().unwrap();

        match awaiting.remove(&sequence_id) {
            Some(sender) => sender.send(response).map_err(|_| ErrorKind::ChannelError)?,
            _ => error!("rx: unexpected response {}", response.command_id()),
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
        while let Some(command) = self.commands.recv().await {
            // TODO: Propagate errors back through the oneshot.
            if let Err(error) = self.process_command(command).await {
                error!("tx: {}", error);
            }
        }

        Ok(())
    }

    async fn process_command(&mut self, command: SerialCommand) -> Result<()> {
        let SerialCommand { request, sender } = command;

        let sequence_id = self.sequence_id();
        let frame = request.into_frame(sequence_id)?;

        self.register_awaiting(sequence_id, sender);
        self.write_frame(frame).await?;

        Ok(())
    }

    fn sequence_id(&mut self) -> SequenceId {
        // Increment by 5 each time, as the Deconz stick seems to ignore some requests if the
        // sequence ID matches the sequence ID of an unsolicited frame.
        let old = self.sequence_id;
        self.sequence_id += 5;
        old
    }

    fn register_awaiting(&self, sequence_id: SequenceId, sender: oneshot::Sender<Response>) {
        self.shared
            .awaiting
            .lock()
            .unwrap()
            .insert(sequence_id, sender);
    }

    async fn write_frame(&mut self, frame: Vec<u8>) -> Result<()> {
        debug!("sending = {:?}", frame);
        self.writer.write_frame(&frame).await?;
        Ok(())
    }
}
