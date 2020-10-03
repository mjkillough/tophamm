mod errors;
mod parameters;
mod protocol;
mod slip;
mod types;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};
use tokio_serial::{Serial, SerialPortSettings};

pub use crate::errors::{Error, ErrorKind, Result};
pub use crate::parameters::{Parameter, ParameterId, PARAMETERS};
pub use crate::protocol::{CommandId, Request, Response};
pub use crate::slip::SlipError;
pub use crate::types::{
    ApsDataIndication, ClusterId, DestinationAddress, DeviceState, Endpoint, ExtendedAddress,
    NetworkState, Platform, ProfileId, SequenceId, ShortAddress, SourceAddress, Version,
};

const BAUD: u32 = 38400;

struct Command {
    request: Request,
    sender: oneshot::Sender<Response>,
}

struct Conbee {
    commands: mpsc::Sender<Command>,
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

        Self { commands }
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

    let fut1 = conbee.make_request(Request::Version);
    let fut2 = conbee.make_request(Request::DeviceState);

    dbg!(fut2.await?);
    dbg!(fut1.await?);

    Ok(())
}
