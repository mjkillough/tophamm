mod errors;
mod parameters;
mod protocol;
mod slip;
mod types;

#[macro_use]
extern crate log;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::stream::{Stream, StreamExt};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_serial::{Serial, SerialPortSettings};

pub use crate::errors::{Error, ErrorKind, Result};
pub use crate::parameters::{Parameter, ParameterId, PARAMETERS};
pub use crate::protocol::{CommandId, Request, Response};
pub use crate::slip::SlipError;
pub use crate::types::{
    ApsDataIndication, ApsDataRequest, ClusterId, Destination, DestinationAddress, DeviceState,
    Endpoint, ExtendedAddress, NetworkState, Platform, ProfileId, SequenceId, ShortAddress,
    SourceAddress, Version,
};

const BAUD: u32 = 38400;

struct Command {
    request: Request,
    sender: oneshot::Sender<Response>,
}

struct Inner {
    commands: mpsc::Sender<Command>,
    request_id: AtomicU8,
}

#[derive(Clone)]
struct Conbee {
    inner: Arc<Inner>,
}

impl Conbee {
    fn new<R, W>(reader: R, writer: W) -> (Self, ApsReader)
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let reader = slip::Reader::new(reader);
        let writer = slip::Writer::new(writer);

        let (commands_tx, commands_rx) = mpsc::channel(1);
        let (device_state_tx, device_state_rx) = watch::channel(DeviceState::default());
        let (aps_data_indications_tx, aps_data_indications_rx) = mpsc::channel(1);

        let request_id = Default::default();
        let conbee = Self {
            inner: Arc::new(Inner {
                commands: commands_tx,
                request_id,
            }),
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
            conbee: conbee.clone(),
            device_state: device_state_rx,
            aps_data_indications: aps_data_indications_tx,
        };

        tokio::spawn(rx.task());
        tokio::spawn(tx.task());
        tokio::spawn(aps.task());

        (conbee, aps_reader)
    }

    async fn make_request(&self, request: Request) -> Result<Response> {
        let (sender, receiver) = oneshot::channel();

        self.inner
            .commands
            .clone()
            .send(Command { request, sender })
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

    fn request_id(&self) -> u8 {
        // Could this be Ordering::Relaxed?
        self.inner.request_id.fetch_add(1, Ordering::SeqCst)
    }

    pub async fn aps_data_request(&self, request: ApsDataRequest) -> Result<()> {
        // TODO: check there are free slots.

        let request_id = self.request_id();
        let request = Request::ApsDataRequest(request_id, request);
        let response = self.make_request(request).await?;

        // We don't bother checking the request_id in the response, as the
        // sequence_id should be sufficient.
        if !matches!(response, Response::ApsDataRequest { .. }) {
            return Err(ErrorKind::UnexpectedResponse(response.command_id()).into());
        }

        // TODO: Wait for confirm.

        Ok(())
    }
}

#[derive(Clone)]
struct Aps {
    conbee: Conbee,
    device_state: watch::Receiver<DeviceState>,
    aps_data_indications: mpsc::Sender<ApsDataIndication>,
    // aps_data_confirms: mpsc::Sender<ApsDataIndication>,
}

impl Aps {
    async fn task(mut self) -> Result<()> {
        while let Some(device_state) = self.device_state.recv().await {
            debug!("aps: {:?}", device_state);

            if device_state.data_indication {
                let mut aps = self.clone();
                tokio::spawn(async move {
                    if let Err(error) = aps.aps_data_indication().await {
                        error!("aps_data_indication: {:?}", error);
                    }
                });
            }
        }

        Ok(())
    }

    async fn aps_data_indication(&mut self) -> Result<()> {
        let response = self.conbee.make_request(Request::ApsDataIndication).await?;
        let aps_data_indication = match response {
            Response::ApsDataIndication {
                aps_data_indication,
                ..
            } => aps_data_indication,
            resp => return Err(ErrorKind::UnexpectedResponse(resp.command_id()).into()),
        };

        self.aps_data_indications
            .send(aps_data_indication)
            .await
            .map_err(|_| ErrorKind::ChannelError)?;

        Ok(())
    }
}

struct ApsReader {
    rx: mpsc::Receiver<ApsDataIndication>,
}

impl Stream for ApsReader {
    type Item = ApsDataIndication;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

#[derive(Default)]
struct Shared {
    awaiting: Mutex<HashMap<SequenceId, oneshot::Sender<Response>>>,
}

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
                error!("rx: {:?}", error);
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
            _ => error!("rx: unexpected response {:?}", response.command_id()),
        }

        Ok(())
    }
}

struct Tx<W>
where
    W: AsyncWrite + Unpin,
{
    shared: Arc<Shared>,
    writer: slip::Writer<W>,
    commands: mpsc::Receiver<Command>,
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
                error!("tx: {:?}", error);
            }
        }

        Ok(())
    }

    async fn process_command(&mut self, command: Command) -> Result<()> {
        let Command { request, sender } = command;

        let sequence_id = self.sequence_id();
        let frame = request.into_frame(sequence_id)?;

        self.register_awaiting(sequence_id, sender);
        self.write_frame(frame).await?;

        Ok(())
    }

    fn sequence_id(&mut self) -> SequenceId {
        // Increment by 5 each time, as the Conbee stick seems to ignore some requests if the
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

#[tokio::main]
async fn main() -> Result<()> {
    pretty_env_logger::init();

    let args = std::env::args().collect::<Vec<_>>();
    let path = &args[1];

    let tty = Serial::from_path(
        path,
        &SerialPortSettings {
            baud_rate: BAUD,
            timeout: std::time::Duration::from_secs(60),
            ..Default::default()
        },
    )?;

    let (reader, writer) = tokio::io::split(tty);
    let (conbee, aps_reader) = Conbee::new(reader, writer);

    // let fut1 = conbee.version();
    // let fut2 = conbee.device_state();
    let fut3 = conbee.aps_data_request(ApsDataRequest {
        destination: Destination::Nwk(345, 0),
        profile_id: 0,
        cluster_id: 0x5,
        source_endpoint: 0,
        asdu: vec![0x0, 0x59, 0x1],
    });

    tokio::spawn(async move {
        let mut aps_reader = aps_reader;
        while let Some(aps_data_indication) = aps_reader.next().await {
            dbg!(aps_data_indication);
        }
    });

    // dbg!(fut2.await?);
    // dbg!(fut1.await?);
    dbg!(fut3.await?);

    loop {}

    Ok(())
}
