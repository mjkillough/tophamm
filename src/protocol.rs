use std::convert::{TryFrom, TryInto};

use byteorder::{ByteOrder, LittleEndian, WriteBytesExt};

use crate::{
    ApsDataIndication, DestinationAddress, DeviceState, NetworkState, Parameter, ParameterId,
    Platform, SequenceId, SourceAddress, Version,
};
use crate::{Error, ErrorKind, Result};

const HEADER_LEN: u16 = 5;

impl From<u8> for Platform {
    fn from(other: u8) -> Self {
        match other {
            0x05 => Platform::Avr,
            0x07 => Platform::Arm,
            unknown => Platform::Unknown(unknown),
        }
    }
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

// TODO: From<u8>
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

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum CommandId {
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

impl CommandId {
    /// Whether a response of this kind was solicited by a request.
    pub fn solicited(&self) -> bool {
        match self {
            CommandId::DeviceStateChanged => false,
            CommandId::ApsDataIndication => false,
            _ => true,
        }
    }
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

#[derive(Debug)]
pub enum Request {
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

impl Request {
    fn command_id(&self) -> CommandId {
        match self {
            Request::Version => CommandId::Version,
            Request::ReadParameter { .. } => CommandId::ReadParameter,
            Request::WriteParameter { .. } => CommandId::WriteParameter,
            Request::DeviceState => CommandId::DeviceState,
            Request::ApsDataIndication => CommandId::ApsDataIndication,
            Request::ApsDataRequest { .. } => CommandId::ApsDataRequest,
        }
    }

    fn payload_len(&self) -> u16 {
        match self {
            Request::Version => 0,
            Request::ReadParameter { .. } => 1,
            Request::WriteParameter { parameter } => 1 + parameter.len(),
            Request::DeviceState => 0,
            Request::ApsDataIndication => 1,
            Request::ApsDataRequest {
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
            Request::Version => {}
            Request::ReadParameter { parameter_id } => {
                buffer.write_u8(parameter_id.into())?;
            }
            Request::WriteParameter { parameter } => {
                buffer.write_u8(parameter.id().into())?;
                parameter.write(buffer)?;
            }
            Request::DeviceState => {}
            Request::ApsDataIndication => {
                buffer.write_u8(4)?;
            }
            Request::ApsDataRequest {
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

impl Request {
    pub fn into_frame(self, sequence_id: SequenceId) -> Result<Vec<u8>> {
        let payload_len = self.payload_len();
        let mut frame_len = HEADER_LEN + payload_len;
        if payload_len > 0 {
            // Only include 2-byte payload length when there is a payload:
            frame_len += 2;
        }

        let mut buffer = Vec::with_capacity(usize::from(frame_len));
        buffer.write_u8(self.command_id().into())?;
        buffer.write_u8(sequence_id)?;
        buffer.write_u8(0)?;
        buffer.write_u16::<LittleEndian>(frame_len)?;

        if payload_len > 0 {
            buffer.write_u16::<LittleEndian>(payload_len)?;
        }

        self.write_payload(&mut buffer)?;

        Ok(buffer)
    }
}

#[derive(Debug)]
pub enum Response {
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

impl Response {
    pub fn from_frame(frame: Vec<u8>) -> Result<(SequenceId, Self)> {
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

                Response::Version { version, platform }
            }
            CommandId::ReadParameter => {
                // Ignore payload length in message:
                let payload = &payload[2..];

                let parameter_id = ParameterId::try_from(payload[0])?;
                let parameter = parameter_id.read_parameter(&payload[1..])?;

                Response::Parameter(parameter)
            }
            CommandId::WriteParameter => {
                // Ignore payload length in message:
                let payload = &payload[2..];

                let parameter_id = ParameterId::try_from(payload[0])?;

                Response::WriteParameter(parameter_id)
            }
            CommandId::DeviceState => {
                let device_state = DeviceState::from_byte(payload[0]);

                Response::DeviceState(device_state)
            }
            CommandId::DeviceStateChanged => {
                let device_state = DeviceState::from_byte(payload[0]);

                Response::DeviceState(device_state)
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

                Response::ApsDataIndication(aps_data_indication)
            }
            CommandId::ApsDataRequest => {
                // Ignore payload length:
                let payload = &payload[2..];

                let device_state = DeviceState::from_byte(payload[0]);
                let request_id = payload[1];

                Response::ApsDataRequest {
                    device_state,
                    request_id,
                }
            }
            CommandId::MacPoll => {
                // Ignore payload length and enum:
                let payload = &payload[3..];

                let address = LittleEndian::read_u16(payload);

                Response::MacPoll { address }
            }
        };

        Ok((sequence_id, kind))
    }
}
