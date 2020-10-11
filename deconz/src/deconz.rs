use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot, watch};
use tophamm_helpers::{awaiting, IncrementingId};

use crate::aps::{self, ApsConfirms, ApsIndications, ApsReader, ApsRequest, ApsRequests};
use crate::protocol::RequestId;
use crate::slip;
use crate::{
    ApsDataConfirm, ApsDataRequest, DeviceState, Error, ErrorKind, Platform, Request, Response,
    Result, SequenceId, Version,
};

/// A command from Deconz to the Tx task, representing a serial Request to be made and the channel
/// tha the response should be sent on.
type SerialCommand = (SequenceId, Request, oneshot::Sender<Result<Response>>);

type Awaiting = awaiting::Awaiting<SequenceId, Response, Error>;

/// Wait for a response to serial commands for at most this amount of time.
const TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Clone)]
pub struct Deconz {
    commands: mpsc::Sender<SerialCommand>,
    aps_data_requests: mpsc::Sender<ApsRequest>,
    sequence_ids: IncrementingId,
    request_ids: IncrementingId,
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
            sequence_ids: IncrementingId::new(),
            request_ids: IncrementingId::new(),
        };
        let aps_reader = ApsReader {
            rx: aps_data_indications_rx,
        };

        let awaiting = Awaiting::new();
        let rx = Rx {
            awaiting: awaiting.clone(),
            reader,
            device_state: device_state_tx,
        };
        let tx = Tx {
            awaiting,
            writer,
            commands: commands_rx,
        };

        let awaiting = aps::Awaiting::new();
        let aps_requests = ApsRequests {
            deconz: deconz.clone(),
            device_state: device_state_rx.clone(),
            awaiting: awaiting.clone(),
            requests: aps_data_requests_rx,
        };
        let aps_confirms = ApsConfirms {
            deconz: deconz.clone(),
            device_state: device_state_rx.clone(),
            awaiting: awaiting.clone(),
        };
        let aps_indications = ApsIndications {
            deconz: deconz.clone(),
            device_state: device_state_rx,
            aps_data_indications: aps_data_indications_tx,
        };

        tokio::spawn(rx.task());
        tokio::spawn(tx.task());
        tokio::spawn(aps_requests.task());
        tokio::spawn(aps_confirms.task());
        tokio::spawn(aps_indications.task());

        (deconz, aps_reader)
    }

    fn sequence_id(&self) -> SequenceId {
        self.sequence_ids.next()
    }

    fn request_id(&self) -> RequestId {
        self.request_ids.next()
    }

    pub async fn make_request(&self, request: Request) -> Result<Response> {
        let (sender, receiver) = oneshot::channel();
        let sequence_id = self.sequence_id();

        self.commands
            .clone()
            .send((sequence_id, request, sender))
            .await
            .map_err(|_| ErrorKind::ChannelError)?;

        let future = tokio::time::timeout(TIMEOUT, receiver);
        let result = future.await?.map_err(|_| ErrorKind::ChannelError)?;
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
        let request_id = self.request_id();

        // Send to Aps task so that it can be sent when the device is ready.
        self.aps_data_requests
            .clone()
            .send((request_id, request, sender))
            .await
            .map_err(|_| ErrorKind::ChannelError)?;

        let result = receiver.await.map_err(|_| ErrorKind::ChannelError)?;
        let aps_data_confirm = result?;

        Ok(aps_data_confirm)
    }
}

/// Task responsible for receiving responses from adapter over serial using the Deconz protocol.
///
/// Forwards responses to futures awaiting a response using the oneshots registered by Tx task.
/// Broadcasts any update to DeviceState for other tasks (e.g. Aps) to respond to.
struct Rx<R>
where
    R: AsyncRead + Unpin,
{
    awaiting: Awaiting,
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
                    error!("rx read_frame: {}", error);
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
        debug!("received frame = {:?}", frame);

        Ok(frame)
    }

    async fn process_frame(&mut self, frame: Vec<u8>) -> Result<()> {
        let sequence_id = frame[1];

        let result = Response::from_frame(frame);
        if let Ok(response) = &result {
            debug!("received response = {:?}", response);

            if let Some(device_state) = response.device_state() {
                let _ = self.device_state.broadcast(device_state);
            }

            // It might just have been a notification from Deconz, in which case we only want to
            // broadcast it.
            if !response.solicited() {
                return Ok(());
            }
        }

        let sender = self
            .awaiting
            .deregister(&sequence_id)
            .ok_or(ErrorKind::UnsolicitedResponse(sequence_id))?;
        let _ = sender.send(result);

        Ok(())
    }
}

/// Task responsible for transmitting requests to adapter over serial using the Deconz protocol.
struct Tx<W>
where
    W: AsyncWrite + Unpin,
{
    awaiting: Awaiting,
    writer: slip::Writer<W>,
    commands: mpsc::Receiver<SerialCommand>,
}

impl<W> Tx<W>
where
    W: AsyncWrite + Unpin,
{
    async fn task(mut self) -> Result<()> {
        while let Some((sequence_id, request, sender)) = self.commands.recv().await {
            let awaiting = self.awaiting.clone();
            let future = self.send_request(sequence_id, request);
            awaiting.register_while(sequence_id, sender, future).await;
        }

        Ok(())
    }

    async fn send_request(&mut self, sequence_id: SequenceId, request: Request) -> Result<()> {
        debug!("sending request = {:?}", request);
        let frame = request.into_frame(sequence_id)?;
        debug!("sending frame = {:?}", frame);
        self.writer.write_frame(&frame).await?;
        Ok(())
    }
}
