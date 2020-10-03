mod errors;
mod parameters;
mod slip;

use std::convert::{TryFrom, TryInto};
use std::pin::Pin;

use byteorder::{ByteOrder, LittleEndian, WriteBytesExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_serial::{Serial, SerialPort, SerialPortSettings};

use crate::parameters::{Parameter, ParameterId, PARAMETERS};
use crate::slip::{Reader, Writer};

pub use crate::errors::{Error, ErrorKind, Result};
pub use crate::slip::SlipError;

const BAUD: u32 = 38400;
const HEADER_LEN: u16 = 5;

#[derive(Copy, Clone, Debug)]
enum Platform {
    Avr,
    Arm,
    Unknown(u8),
}

impl From<u8> for Platform {
    fn from(other: u8) -> Self {
        match other {
            0x05 => Platform::Avr,
            0x07 => Platform::Arm,
            unknown => Platform::Unknown(unknown),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Version {
    major: u8,
    minor: u8,
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum CommandId {
    Version,
    ReadParameter,
    WriteParameter,
    DeviceState,
    DeviceStateChanged,
    ApsDataIndication,
    ApsDataRequest,

    // https://github.com/dresden-elektronik/deconz-rest-plugin/issues/652#issuecomment-400055215
    MacPoll,
}

impl From<CommandId> for u8 {
    fn from(command_id: CommandId) -> u8 {
        match command_id {
            CommandId::Version => 0x0D,
            CommandId::ReadParameter => 0x0A,
            CommandId::WriteParameter => 0x0B,
            CommandId::DeviceState => 0x07,
            CommandId::DeviceStateChanged => 0x0E,
            CommandId::ApsDataIndication => 0x17,
            CommandId::ApsDataRequest => 0x12,
            CommandId::MacPoll => 0x1C,
        }
    }
}

impl TryFrom<u8> for CommandId {
    type Error = Error;

    fn try_from(byte: u8) -> Result<Self> {
        match byte {
            0x0D => Ok(CommandId::Version),
            0x0A => Ok(CommandId::ReadParameter),
            0x0B => Ok(CommandId::WriteParameter),
            0x07 => Ok(CommandId::DeviceState),
            0x0E => Ok(CommandId::DeviceStateChanged),
            0x1C => Ok(CommandId::MacPoll),
            0x17 => Ok(CommandId::ApsDataIndication),
            0x12 => Ok(CommandId::ApsDataRequest),
            _ => Err(Error {
                kind: ErrorKind::UnsupportedCommand(byte),
            }),
        }
    }
}

type SequenceId = u8;

#[derive(Copy, Clone, Debug)]
enum NetworkState {
    Offline,
    Joining,
    Connected,
    Leaving,
}

impl From<u8> for NetworkState {
    fn from(byte: u8) -> Self {
        match byte {
            0x0 => NetworkState::Offline,
            0x1 => NetworkState::Joining,
            0x2 => NetworkState::Connected,
            0x3 => NetworkState::Leaving,
            _ => unreachable!("we only ever parse 2 bits"),
        }
    }
}

#[derive(Copy, Clone, Debug)]
struct DeviceState {
    network_state: NetworkState,
    data_confirm: bool,
    data_indication: bool,
    data_request_free_slots: bool,
    configuration_changed: bool,
}

impl DeviceState {
    fn from_byte(byte: u8) -> Self {
        let network_state = (byte & 0b11).into();
        let data_confirm = (byte & 0b100) > 0;
        let data_indication = (byte & 0b1000) > 0;
        let data_request_free_slots = (byte & 0b100000) > 0;
        let configuration_changed = (byte & 0b10000) > 0;
        Self {
            network_state,
            data_confirm,
            data_indication,
            data_request_free_slots,
            configuration_changed,
        }
    }
}

#[derive(Debug)]
enum RequestKind {
    Version,
    ReadParameter {
        parameter_id: ParameterId,
    },
    WriteParameter {
        parameter: Parameter,
    },
    DeviceState,
    ApsDataIndication,
    ApsDataRequest {
        request_id: u8,
        destination_address: DestinationAddress,
        destination_endpoint: Option<u8>,
        profile_id: u16,
        cluster_id: u16,
        source_endpoint: u8,
        asdu: Vec<u8>,
    },
}

impl RequestKind {
    fn command_id(&self) -> CommandId {
        match self {
            RequestKind::Version => CommandId::Version,
            RequestKind::ReadParameter { .. } => CommandId::ReadParameter,
            RequestKind::WriteParameter { .. } => CommandId::WriteParameter,
            RequestKind::DeviceState => CommandId::DeviceState,
            RequestKind::ApsDataIndication => CommandId::ApsDataIndication,
            RequestKind::ApsDataRequest { .. } => CommandId::ApsDataRequest,
        }
    }

    fn payload_len(&self) -> u16 {
        match self {
            RequestKind::Version => 0,
            RequestKind::ReadParameter { .. } => 1,
            RequestKind::WriteParameter { parameter } => 1 + parameter.len(),
            RequestKind::DeviceState => 0,
            RequestKind::ApsDataIndication => 1,
            RequestKind::ApsDataRequest {
                destination_address,
                asdu,
                ..
            } => {
                let base = 12;
                let addr = match destination_address {
                    DestinationAddress::Group(_) => 2, // short address
                    DestinationAddress::Nwk(_) => 3,   // short address + endpoint
                    DestinationAddress::Ieee(_) => 9,  // ieee address + endpoint
                };
                base + addr + (asdu.len() as u16)
            }
        }
    }

    fn write_payload(self, buffer: &mut Vec<u8>) -> Result<()> {
        match self {
            RequestKind::Version => {}
            RequestKind::ReadParameter { parameter_id } => {
                buffer.write_u8(parameter_id.into())?;
            }
            RequestKind::WriteParameter { parameter } => {
                buffer.write_u8(parameter.id().into())?;
                parameter.write(buffer)?;
            }
            RequestKind::DeviceState => {}
            RequestKind::ApsDataIndication => {
                buffer.write_u8(4)?;
            }
            RequestKind::ApsDataRequest {
                request_id,
                destination_address,
                destination_endpoint,
                profile_id,
                cluster_id,
                source_endpoint,
                asdu,
            } => {
                buffer.write_u8(request_id)?;
                buffer.write_u8(0)?; // flags
                match destination_address {
                    DestinationAddress::Group(addr) => {
                        buffer.write_u8(0x01)?; // addr type
                        buffer.write_u16::<LittleEndian>(addr)?;
                    }
                    DestinationAddress::Nwk(addr) => {
                        buffer.write_u8(0x02)?; // addr type
                        buffer.write_u16::<LittleEndian>(addr)?;
                        buffer.write_u8(destination_endpoint.ok_or(Error {
                            kind: ErrorKind::Todo,
                        })?)?;
                    }
                    DestinationAddress::Ieee(addr) => {
                        buffer.write_u8(0x03)?; // addr type
                        buffer.write_u64::<LittleEndian>(addr)?;
                        buffer.write_u8(destination_endpoint.ok_or(Error {
                            kind: ErrorKind::Todo,
                        })?)?;
                    }
                }
                buffer.write_u16::<LittleEndian>(profile_id)?;
                buffer.write_u16::<LittleEndian>(cluster_id)?;
                buffer.write_u8(source_endpoint)?;
                buffer.write_u16::<LittleEndian>(asdu.len() as u16)?;
                buffer.extend(asdu);
                buffer.write_u8(0x04)?; // tx options, use aps acks
                buffer.write_u8(0)?; // radius, infinite hops
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
struct Request {
    sequence_id: SequenceId,
    kind: RequestKind,
}

impl Request {
    fn into_frame(self) -> Result<Vec<u8>> {
        let payload_len = self.kind.payload_len();
        let mut frame_len = HEADER_LEN + payload_len;
        if payload_len > 0 {
            // Only include 2-byte payload length when there is a payload:
            frame_len += 2;
        }

        let mut buffer = Vec::with_capacity(usize::from(frame_len));
        buffer.write_u8(self.kind.command_id().into())?;
        buffer.write_u8(self.sequence_id)?;
        buffer.write_u8(0)?;
        buffer.write_u16::<LittleEndian>(frame_len)?;

        if payload_len > 0 {
            buffer.write_u16::<LittleEndian>(payload_len)?;
        }

        self.kind.write_payload(&mut buffer)?;

        Ok(buffer)
    }
}

#[derive(Debug)]
enum ResponseKind {
    Version {
        version: Version,
        platform: Platform,
    },
    Parameter(Parameter),
    WriteParameter(ParameterId),
    DeviceState(DeviceState),
    DeviceStateChanged(DeviceState),
    ApsDataIndication(ApsDataIndication),
    ApsDataRequest {
        device_state: DeviceState,
        request_id: u8,
    },
    MacPoll {
        address: u16,
    },
}

#[derive(Debug)]
struct Response {
    sequence_id: SequenceId,
    kind: ResponseKind,
}

impl Response {
    fn from_frame(frame: Vec<u8>) -> Result<Self> {
        let command_id = frame[0].try_into()?;
        let sequence_id = frame[1];

        let header_len: usize = HEADER_LEN.into();
        let frame_len: usize = LittleEndian::read_u16(&frame[3..]).into();
        let payload_len = frame_len - header_len;
        let payload = &frame[header_len..];

        debug_assert!(payload.len() == payload_len);

        let kind = match command_id {
            CommandId::Version => {
                let platform = payload[1].into();
                let minor = payload[2];
                let major = payload[3];

                let version = Version { major, minor };

                ResponseKind::Version { version, platform }
            }
            CommandId::ReadParameter => {
                // Ignore payload length in message:
                let payload = &payload[2..];

                let parameter_id = ParameterId::try_from(payload[0])?;
                let parameter = parameter_id.read_parameter(&payload[1..])?;

                ResponseKind::Parameter(parameter)
            }
            CommandId::WriteParameter => {
                // Ignore payload length in message:
                let payload = &payload[2..];

                let parameter_id = ParameterId::try_from(payload[0])?;

                ResponseKind::WriteParameter(parameter_id)
            }
            CommandId::DeviceState => {
                let device_state = DeviceState::from_byte(payload[0]);

                ResponseKind::DeviceState(device_state)
            }
            CommandId::DeviceStateChanged => {
                let device_state = DeviceState::from_byte(payload[0]);

                ResponseKind::DeviceState(device_state)
            }
            CommandId::ApsDataIndication => {
                use byteorder::ReadBytesExt;
                use std::io::{Cursor, Read};

                let mut payload = Cursor::new(payload);

                let _payload_len = payload.read_u16::<LittleEndian>()?;

                let device_state = DeviceState::from_byte(payload.read_u8()?);
                let destination_address = match payload.read_u8()? {
                    0x1 => DestinationAddress::Group(payload.read_u16::<LittleEndian>()?),
                    0x2 => DestinationAddress::Nwk(payload.read_u16::<LittleEndian>()?),
                    0x3 => DestinationAddress::Ieee(payload.read_u64::<LittleEndian>()?),
                    _ => unimplemented!("unknown destination address mode"),
                };
                let destination_endpoint = payload.read_u8()?;

                let source_address = match payload.read_u8()? {
                    0x4 => {
                        let short = payload.read_u16::<LittleEndian>()?;
                        let extended = payload.read_u64::<LittleEndian>()?;
                        SourceAddress { short, extended }
                    }
                    _ => unimplemented!("unknown source address mode "),
                };
                let source_endpoint = payload.read_u8()?;

                let profile_id = payload.read_u16::<LittleEndian>()?;
                let cluster_id = payload.read_u16::<LittleEndian>()?;

                let asdu_length = payload.read_u16::<LittleEndian>()?;
                let mut asdu = vec![0; asdu_length.into()];
                payload.read(&mut asdu)?;

                let aps_data_indication = ApsDataIndication {
                    device_state,
                    destination_address,
                    destination_endpoint,
                    source_address,
                    source_endpoint,
                    profile_id,
                    cluster_id,
                    asdu,
                };

                ResponseKind::ApsDataIndication(aps_data_indication)
            }
            CommandId::ApsDataRequest => {
                // Ignore payload length:
                let payload = &payload[2..];

                let device_state = DeviceState::from_byte(payload[0]);
                let request_id = payload[1];

                ResponseKind::ApsDataRequest {
                    device_state,
                    request_id,
                }
            }
            CommandId::MacPoll => {
                // Ignore payload length and enum:
                let payload = &payload[3..];

                let address = LittleEndian::read_u16(payload);

                ResponseKind::MacPoll { address }
            }
        };

        Ok(Response { sequence_id, kind })
    }
}

type ShortAddress = u16;
type ExtendedAddress = u64;

#[derive(Debug)]
enum DestinationAddress {
    Group(ShortAddress),
    Nwk(ShortAddress),
    Ieee(ExtendedAddress),
}

#[derive(Debug)]
struct SourceAddress {
    short: ShortAddress,
    extended: ExtendedAddress,
}

#[derive(Debug)]
struct ApsDataIndication {
    device_state: DeviceState,
    destination_address: DestinationAddress,
    destination_endpoint: u8,
    source_address: SourceAddress,
    source_endpoint: u8,
    profile_id: u16,
    cluster_id: u16,
    asdu: Vec<u8>,
}

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

    Ok(())
}
