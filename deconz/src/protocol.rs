use std::convert::{TryFrom, TryInto};
use std::fmt::{self, Display};
use std::io::{Cursor, Read, Write};

use crate::{
    ApsDataConfirm, ApsDataIndication, ApsDataRequest, Destination, DestinationAddress,
    DeviceState, NetworkState, Parameter, ParameterId, Platform, ReadWire, SequenceId,
    SourceAddress, Version, WriteWire,
};
use crate::{Error, ErrorKind, ReadWireExt, Result, WriteWireExt};

const HEADER_LEN: u16 = 5;

impl ReadWire for Platform {
    fn read_wire<R>(r: &mut R) -> Result<Self>
    where
        R: Read,
    {
        let byte = u8::read_wire(r)?;

        let platform = match byte {
            0x05 => Platform::Avr,
            0x07 => Platform::Arm,
            unknown => Platform::Unknown(unknown),
        };

        Ok(platform)
    }
}

impl ReadWire for Version {
    fn read_wire<R>(r: &mut R) -> Result<Self>
    where
        R: Read,
    {
        let minor = r.read_wire()?;
        let major = r.read_wire()?;

        Ok(Version { minor, major })
    }
}

impl ReadWire for DeviceState {
    fn read_wire<R>(r: &mut R) -> Result<Self>
    where
        R: Read,
    {
        let byte = u8::read_wire(r)?;

        let network_state = match byte & 0b11 {
            0x0 => NetworkState::Offline,
            0x1 => NetworkState::Joining,
            0x2 => NetworkState::Connected,
            0x3 => NetworkState::Leaving,
            _ => unreachable!("we only ever parse 2 bits"),
        };
        let data_confirm = (byte & 0b100) > 0;
        let data_indication = (byte & 0b1000) > 0;
        let data_request_free_slots = (byte & 0b100000) > 0;
        let configuration_changed = (byte & 0b10000) > 0;

        Ok(Self {
            network_state,
            data_confirm,
            data_indication,
            data_request_free_slots,
            configuration_changed,
        })
    }
}

impl ReadWire for Destination {
    fn read_wire<R>(r: &mut R) -> Result<Self>
    where
        R: Read,
    {
        match u8::read_wire(r)? {
            0x1 => Ok(Destination::Group(r.read_wire()?)),
            0x2 => {
                let short_address = r.read_wire()?;
                let endpoint = r.read_wire()?;
                Ok(Destination::Nwk(short_address, endpoint))
            }
            0x3 => {
                let extended_address = r.read_wire()?;
                let endpoint = r.read_wire()?;
                Ok(Destination::Ieee(extended_address, endpoint))
            }
            _ => unreachable!("invalid address mode"),
        }
    }
}

impl WriteWire for Destination {
    fn wire_len(&self) -> u16 {
        match self {
            Destination::Group(_) => 2,
            Destination::Nwk(_, _) => 3,
            Destination::Ieee(_, _) => 9,
        }
    }

    fn write_wire<W>(self, w: &mut W) -> Result<()>
    where
        W: Write,
    {
        // Address mode
        let address_mode: u8 = match self {
            Destination::Group(_) => 0x1,
            Destination::Nwk(_, _) => 0x2,
            Destination::Ieee(_, _) => 0x3,
        };
        w.write_wire(address_mode)?;

        // Address
        match self {
            Destination::Group(addr) | Destination::Nwk(addr, _) => {
                w.write_wire(addr)?;
            }
            Destination::Ieee(addr, _) => {
                w.write_wire(addr)?;
            }
        };

        // Endpoint
        match self {
            Destination::Group(_) => {}
            Destination::Nwk(_, endpoint) | Destination::Ieee(_, endpoint) => {
                w.write_wire(endpoint)?;
            }
        }

        Ok(())
    }
}

pub type RequestId = u8;

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum CommandId {
    Version,
    ReadParameter,
    WriteParameter,
    DeviceState,
    DeviceStateChanged,
    ApsDataIndication,
    ApsDataRequest,
    ApsDataConfirm,

    // https://github.com/dresden-elektronik/deconz-rest-plugin/issues/652#issuecomment-400055215
    MacPoll,
}

impl CommandId {
    /// Whether a response of this kind was solicited by a request.
    pub fn solicited(&self) -> bool {
        match self {
            CommandId::DeviceStateChanged => false,
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
            CommandId::ApsDataConfirm => 0x04,
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
            0x04 => Ok(CommandId::ApsDataConfirm),
            _ => Err(Error {
                kind: ErrorKind::UnsupportedCommand(byte),
            }),
        }
    }
}

impl ReadWire for CommandId {
    fn read_wire<R>(r: &mut R) -> Result<Self>
    where
        R: Read,
    {
        let byte: u8 = r.read_wire()?;
        byte.try_into()
    }
}

impl WriteWire for CommandId {
    fn wire_len(&self) -> u16 {
        1
    }

    fn write_wire<W>(self, w: &mut W) -> Result<()>
    where
        W: Write,
    {
        w.write_wire(u8::from(self))?;
        Ok(())
    }
}

impl Display for CommandId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandId::Version => write!(f, "Version ({})", u8::from(*self)),
            CommandId::ReadParameter => write!(f, "ReadParameter ({})", u8::from(*self)),
            CommandId::WriteParameter => write!(f, "WriteParameter ({})", u8::from(*self)),
            CommandId::DeviceState => write!(f, "DeviceState ({})", u8::from(*self)),
            CommandId::DeviceStateChanged => write!(f, "DeviceStateChanged ({})", u8::from(*self)),
            CommandId::MacPoll => write!(f, "MacPoll ({})", u8::from(*self)),
            CommandId::ApsDataIndication => write!(f, "ApsDataIndication ({})", u8::from(*self)),
            CommandId::ApsDataRequest => write!(f, "ApsDataRequest ({})", u8::from(*self)),
            CommandId::ApsDataConfirm => write!(f, "ApsDataConfirm ({})", u8::from(*self)),
        }
    }
}

#[derive(Debug)]
pub enum Request {
    Version,
    ReadParameter { parameter_id: ParameterId },
    WriteParameter { parameter: Parameter },
    DeviceState,
    ApsDataIndication,
    ApsDataRequest(RequestId, ApsDataRequest),
    ApsDataConfirm,
}

impl Request {
    fn command_id(&self) -> CommandId {
        match self {
            Request::Version => CommandId::Version,
            Request::ReadParameter { .. } => CommandId::ReadParameter,
            Request::WriteParameter { .. } => CommandId::WriteParameter,
            Request::DeviceState => CommandId::DeviceState,
            Request::ApsDataIndication => CommandId::ApsDataIndication,
            Request::ApsDataRequest(_, _) => CommandId::ApsDataRequest,
            Request::ApsDataConfirm => CommandId::ApsDataConfirm,
        }
    }

    fn payload_len(&self) -> Option<u16> {
        match self {
            Request::Version => None,
            Request::ReadParameter { .. } => Some(1),
            Request::WriteParameter { parameter } => Some(1 + parameter.wire_len()),
            Request::DeviceState => None,
            Request::ApsDataIndication => Some(1),
            Request::ApsDataRequest(
                _,
                ApsDataRequest {
                    destination, asdu, ..
                },
            ) => Some(12 + destination.wire_len() + (asdu.len() as u16)),
            // Include payload len even though it is zero:
            Request::ApsDataConfirm => Some(0),
        }
    }

    fn write_payload(self, buffer: &mut Vec<u8>) -> Result<()> {
        match self {
            Request::Version => {}
            Request::ReadParameter { parameter_id } => {
                buffer.write_wire(parameter_id)?;
            }
            Request::WriteParameter { parameter } => {
                buffer.write_wire(parameter.id())?;
                buffer.write_wire(parameter)?;
            }
            Request::DeviceState => {}
            Request::ApsDataIndication => {
                buffer.write_wire(4 as u8)?;
            }
            Request::ApsDataRequest(
                request_id,
                ApsDataRequest {
                    destination,
                    profile_id,
                    cluster_id,
                    source_endpoint,
                    asdu,
                },
            ) => {
                buffer.write_wire(request_id)?;
                buffer.write_wire(0 as u8)?; // flags
                buffer.write_wire(destination)?;
                buffer.write_wire(profile_id)?;
                buffer.write_wire(cluster_id)?;
                buffer.write_wire(source_endpoint)?;
                buffer.write_wire(asdu.len() as u16)?;
                buffer.extend(asdu);
                buffer.write_wire(0x04 as u8)?; // tx options, use aps acks
                buffer.write_wire(0 as u8)?; // radius, infinite hops
            }
            Request::ApsDataConfirm => {}
        }

        Ok(())
    }
}

impl Request {
    pub fn into_frame(self, sequence_id: SequenceId) -> Result<Vec<u8>> {
        let payload_len = self.payload_len();
        let mut frame_len = HEADER_LEN;
        if let Some(payload_len) = payload_len {
            // Only include 2-byte payload length when there is a payload:
            // 2 byte payload len:
            frame_len += 2;
            // Payload:
            frame_len += payload_len;
        }

        let mut buffer = Vec::with_capacity(usize::from(frame_len));
        buffer.write_wire(self.command_id())?;
        buffer.write_wire(sequence_id)?;
        buffer.write_wire(0 as u8)?;
        buffer.write_wire(frame_len)?;

        if let Some(payload_len) = payload_len {
            buffer.write_wire(payload_len)?;
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
    ApsDataIndication {
        device_state: DeviceState,
        aps_data_indication: ApsDataIndication,
    },
    ApsDataRequest {
        device_state: DeviceState,
        request_id: RequestId,
    },
    ApsDataConfirm {
        device_state: DeviceState,
        request_id: RequestId,
        aps_data_confirm: ApsDataConfirm,
    },
    MacPoll {
        address: u16,
    },
}

impl Response {
    pub fn command_id(&self) -> CommandId {
        match self {
            Response::Version { .. } => CommandId::Version,
            Response::Parameter(_) => CommandId::ReadParameter,
            Response::WriteParameter(_) => CommandId::WriteParameter,
            Response::DeviceState(_) => CommandId::DeviceState,
            Response::DeviceStateChanged(_) => CommandId::DeviceStateChanged,
            Response::ApsDataIndication { .. } => CommandId::ApsDataIndication,
            Response::ApsDataRequest { .. } => CommandId::ApsDataRequest,
            Response::ApsDataConfirm { .. } => CommandId::ApsDataConfirm,
            Response::MacPoll { .. } => CommandId::MacPoll,
        }
    }

    pub fn solicited(&self) -> bool {
        self.command_id().solicited()
    }

    pub fn device_state(&self) -> Option<DeviceState> {
        match self {
            Response::DeviceState(device_state)
            | Response::DeviceStateChanged(device_state)
            | Response::ApsDataIndication { device_state, .. }
            | Response::ApsDataRequest { device_state, .. } => Some(*device_state),
            _ => None,
        }
    }

    pub fn from_frame(frame: Vec<u8>) -> Result<(SequenceId, Self)> {
        let len = frame.len();
        let mut frame = Cursor::new(frame);

        let command_id = frame.read_wire()?;
        let sequence_id = frame.read_wire()?;

        let _reserved: u8 = frame.read_wire()?;

        let header_len: usize = HEADER_LEN.into();
        let frame_len: u16 = frame.read_wire()?;
        let payload_len = usize::from(frame_len) - header_len;

        debug_assert!(len - header_len == payload_len);

        let mut payload = frame;

        let kind = match command_id {
            CommandId::Version => {
                let platform = payload.read_wire()?;
                let version = payload.read_wire()?;

                Response::Version { version, platform }
            }
            CommandId::ReadParameter => {
                let _payload_len: u16 = payload.read_wire()?;

                let parameter_id: ParameterId = payload.read_wire()?;
                let parameter = parameter_id.read_parameter(&mut payload)?;

                Response::Parameter(parameter)
            }
            CommandId::WriteParameter => {
                let _payload_len: u16 = payload.read_wire()?;

                let parameter_id = payload.read_wire()?;

                Response::WriteParameter(parameter_id)
            }
            CommandId::DeviceState => {
                let device_state = payload.read_wire()?;

                Response::DeviceState(device_state)
            }
            CommandId::DeviceStateChanged => {
                let device_state = payload.read_wire()?;

                Response::DeviceStateChanged(device_state)
            }
            CommandId::ApsDataIndication => {
                let _payload_len: u16 = payload.read_wire()?;

                let device_state = payload.read_wire()?;
                let destination_address = match u8::read_wire(&mut payload)? {
                    0x1 => DestinationAddress::Group(payload.read_wire()?),
                    0x2 => DestinationAddress::Nwk(payload.read_wire()?),
                    0x3 => DestinationAddress::Ieee(payload.read_wire()?),
                    _ => unimplemented!("unknown destination address mode"),
                };
                let destination_endpoint = payload.read_wire()?;

                let source_address = match u8::read_wire(&mut payload)? {
                    0x4 => {
                        let short = payload.read_wire()?;
                        let extended = payload.read_wire()?;
                        SourceAddress { short, extended }
                    }
                    _ => unimplemented!("unknown source address mode "),
                };
                let source_endpoint = payload.read_wire()?;

                let profile_id = payload.read_wire()?;
                let cluster_id = payload.read_wire()?;

                let asdu_length: u16 = payload.read_wire()?;
                let mut asdu = vec![0; asdu_length.into()];
                payload.read(&mut asdu)?;

                let aps_data_indication = ApsDataIndication {
                    destination_address,
                    destination_endpoint,
                    source_address,
                    source_endpoint,
                    profile_id,
                    cluster_id,
                    asdu,
                };

                Response::ApsDataIndication {
                    device_state,
                    aps_data_indication,
                }
            }
            CommandId::ApsDataRequest => {
                let _payload_len: u16 = payload.read_wire()?;

                let device_state = payload.read_wire()?;
                let request_id = payload.read_wire()?;

                Response::ApsDataRequest {
                    device_state,
                    request_id,
                }
            }
            CommandId::MacPoll => {
                let _payload_len: u16 = payload.read_wire()?;
                let _enum: u8 = payload.read_wire()?;

                let address = payload.read_wire()?;

                Response::MacPoll { address }
            }
            CommandId::ApsDataConfirm => {
                let _payload_len: u16 = payload.read_wire()?;

                let device_state = payload.read_wire()?;
                let request_id = payload.read_wire()?;
                let destination = payload.read_wire()?;
                let source_endpoint = payload.read_wire()?;
                let status = payload.read_wire()?;

                let aps_data_confirm = ApsDataConfirm {
                    destination,
                    source_endpoint,
                    status,
                };

                Response::ApsDataConfirm {
                    device_state,
                    request_id,
                    aps_data_confirm,
                }
            }
        };

        Ok((sequence_id, kind))
    }
}
