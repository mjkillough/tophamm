mod errors;
mod parameters;
mod protocol;
mod slip;
mod types;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};
use tokio_serial::{Serial, SerialPortSettings};

use crate::protocol::RequestId;

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

struct Conbee {
    commands: mpsc::Sender<Command>,
    request_id: AtomicU8,
}

impl Conbee {
    fn new<R, W>(reader: R, writer: W) -> Self
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let reader = slip::Reader::new(reader);
        let writer = slip::Writer::new(writer);

        let (commands, commands_rx) = mpsc::channel(1);
        let request_id = Default::default();

        let shared = Arc::new(Shared::default());
        let rx = Rx {
            shared: shared.clone(),
            reader,
        };
        let tx = Tx {
            shared,
            writer,
            commands: commands_rx,
            sequence_id: 0,
        };

        tokio::spawn(rx.task());
        tokio::spawn(tx.task());

        Self {
            commands,
            request_id,
        }
    }

    async fn make_request(&self, request: Request) -> Result<Response> {
        let (sender, receiver) = oneshot::channel();

        self.commands
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
        self.request_id.fetch_add(1, Ordering::SeqCst)
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
}

impl<R> Rx<R>
where
    R: AsyncRead + Unpin,
{
    async fn task(mut self) -> Result<()> {
        loop {
            let frame = self.reader.read_frame().await?;
            println!("received = {:?}", frame);

            // TODO: Think about where to send errors
            let (sequence_id, response) = Response::from_frame(frame)?;

            if let Some(sender) = self.shared.awaiting.lock().unwrap().remove(&sequence_id) {
                sender.send(response).map_err(|_| ErrorKind::ChannelError)?;
            } else {
                println!("don't know where to send frame");
            }
        }
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
        while let Some(Command { request, sender }) = self.commands.recv().await {
            // TODO: Propagate errors back through the oneshot.
            let sequence_id = self.sequence_id();
            let frame = request.into_frame(sequence_id)?;

            self.register_awaiting(sequence_id, sender);

            self.write_frame(frame).await?;
        }

        Ok(())
    }

    fn sequence_id(&mut self) -> SequenceId {
        let old = self.sequence_id;
        self.sequence_id += 1;
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
        println!("sending = {:?}", frame);
        self.writer.write_frame(&frame).await?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
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
    let conbee = Conbee::new(reader, writer);

    let fut1 = conbee.version();
    let fut2 = conbee.device_state();
    let fut3 = conbee.aps_data_request(ApsDataRequest {
        destination: Destination::Nwk(345, 0),
        profile_id: 0,
        cluster_id: 0x5,
        source_endpoint: 0,
        asdu: vec![0x0, 0x59, 0x1],
    });

    dbg!(fut2.await?);
    dbg!(fut1.await?);
    dbg!(fut3.await?);

    Ok(())
}
