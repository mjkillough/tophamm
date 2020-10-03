mod errors;
mod parameters;
mod protocol;
mod slip;
mod types;

use byteorder::{ByteOrder, LittleEndian};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_serial::{Serial, SerialPortSettings};

use crate::slip::{Reader, Writer};

pub use crate::errors::{Error, ErrorKind, Result};
pub use crate::parameters::{Parameter, ParameterId, PARAMETERS};
pub use crate::protocol::{CommandId, Request, RequestKind, Response, ResponseKind};
pub use crate::slip::SlipError;
pub use crate::types::{
    ApsDataIndication, ClusterId, DestinationAddress, DeviceState, Endpoint, ExtendedAddress,
    NetworkState, Platform, ProfileId, SequenceId, ShortAddress, SourceAddress, Version,
};

const BAUD: u32 = 38400;

struct Conbee<R, W>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    reader: Reader<R>,
    writer: Writer<W>,
}

impl<R, W> Conbee<R, W>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    async fn make_request(&mut self, request: Request) -> Result<Response> {
        println!("request = {:?}", request);
        let frame = request.into_frame()?;
        println!("sending = {:?}", frame);
        self.writer.write_frame(&frame).await?;

        println!("waiting for resp");

        let response = self.receive_response().await?;

        Ok(response)
    }

    async fn receive_response(&mut self) -> Result<Response> {
        let frame = self.reader.read_frame().await?;
        println!("received = {:?}", frame);
        let response = Response::from_frame(frame)?;
        println!("response = {:?}", response);
        Ok(response)
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
    let reader = Reader::new(reader);
    let writer = Writer::new(writer);

    let mut conbee = Conbee { reader, writer };

    let mut sequence_id = 1;

    conbee
        .make_request(Request {
            sequence_id,
            kind: RequestKind::Version,
        })
        .await?;
    sequence_id += 1;

    // conbee.make_request(Request {
    //     sequence_id,
    //     kind: RequestKind::DeviceState,
    // })?;
    // sequence_id += 1;

    // conbee.make_request(Request {
    //     sequence_id,
    //     kind: RequestKind::WriteParameter {
    //         parameter: Parameter::NetworkKey(0x01),
    //     },
    // })?;
    // sequence_id += 1;

    // conbee.make_request(Request {
    //     sequence_id,
    //     kind: RequestKind::ReadParameter {
    //         parameter_id: ParameterId::NetworkKey,
    //     },
    // })?;
    // sequence_id += 1;

    let mut buf = [0; 3];
    LittleEndian::write_u16(&mut buf[1..], 345);
    conbee
        .make_request(Request {
            sequence_id,
            kind: RequestKind::ApsDataRequest {
                request_id: 1,
                destination_address: DestinationAddress::Nwk(345),
                destination_endpoint: Some(0),
                profile_id: 0,
                cluster_id: 5,
                source_endpoint: 0,
                asdu: buf.to_vec(),
            },
        })
        .await?;
    sequence_id += 1;

    let mut response = conbee
        .make_request(Request {
            sequence_id,
            kind: RequestKind::DeviceState,
        })
        .await?;
    sequence_id += 1;
    loop {
        let data_indication = match response.kind {
            ResponseKind::DeviceState(device_state)
            | ResponseKind::DeviceStateChanged(device_state) => device_state.data_indication,
            _ => false,
        };

        if dbg!(data_indication) {
            conbee
                .make_request(Request {
                    sequence_id,
                    kind: RequestKind::ApsDataIndication,
                })
                .await?;
            sequence_id += 1;

            response = conbee
                .make_request(Request {
                    sequence_id,
                    kind: RequestKind::DeviceState,
                })
                .await?;
            sequence_id += 1;
        } else {
            // Wait for unsolicited DeviceStateChanged
            response = conbee.receive_response().await?;
        }
    }
}
