use std::fmt::{self, Display};
use std::io::{Read, Write};

use crate::{Error, ErrorKind, ReadWire, ReadWireExt, Result, WriteWire, WriteWireExt};

macro_rules! define_parameters {
    ($(($param:ident, $id:expr, $ty:ty)),+ $(,)?) => {
        pub const PARAMETERS: &[ParameterId] = &[$(ParameterId::$param),+];

        #[derive(Copy, Clone, Debug)]
        pub enum ParameterId {
            $($param),+
        }

        #[derive(Copy, Clone, Debug)]
        pub enum Parameter {
            $($param($ty)),+
        }

        impl Parameter {
            pub fn id(&self) -> ParameterId {
                match self {
                    $(Parameter::$param(_) => ParameterId::$param),+
                }
            }
        }

        impl WriteWire for Parameter {
            fn wire_len(&self) -> u16 {
                match self {
                    $(Parameter::$param(_) => std::mem::size_of::<$ty>() as u16),+
                }
            }

            fn write_wire<W>(self, w: &mut W) -> Result<()> where W: Write {
                match self {
                    $(Parameter::$param(value) => w.write_wire(value)),+
                 }
            }
        }

        impl ParameterId {
            pub fn read_parameter<R>(&self, r: &mut R) -> Result<Parameter>
                where R: Read,
            {
                match self {
                    $(
                        ParameterId::$param => {
                            let param = r.read_wire()
                                .map_err(|err| {
                                    Error {
                                        kind: ErrorKind::InvalidParameter {
                                            parameter_id: *self,
                                            inner: Box::new(err),
                                        }
                                    }
                                })?;
                            Ok(Parameter::$param(param))
                        }
                    )+
                }
            }
        }

        impl Display for ParameterId {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self {
                    $(ParameterId::$param => write!(f, "{} ({})", stringify!($param), $id)),+
                }
            }
        }

        impl ReadWire for ParameterId {
            fn read_wire<R>(r: &mut R) -> Result<Self>
                where
                    R: Read,
            {
                let byte = u8::read_wire(r)?;
                match byte {
                    $($id => Ok(ParameterId::$param),)+
                    _ => Err(Error { kind: ErrorKind::UnsupportedParameter(byte) }),
                }
            }
        }

        impl WriteWire for ParameterId {
            fn wire_len(&self) -> u16 {
                1
            }

            fn write_wire<W>(self, w: &mut W) -> Result<()>
            where
                W: Write,
            {
                let byte: u8 = match self {
                    $(ParameterId::$param => $id,)+
                };
                w.write_wire(byte)?;
                Ok(())
            }
        }
    };
}

define_parameters! {
    (MacAddress, 0x01, u64),
    (NwkPanId, 0x05, u16),
    (NwkAddress, 0x07, u16),
    (NwkExtendedPanId, 0x08, u64),
    (ApsDesignatedCoordinator, 0x09, u8),
    (ChannelMask, 0x0A, u32),
    (ApsExtendedPanId, 0x0B, u64),
    (TrustCenterAddress, 0x0E, u64),
    (SecurityMode, 0x10, u8),
    (NetworkKey, 0x18, u8),
    (CurrentChannel, 0x1C, u8),
    (ProtocolVersion, 0x22, u16),
    (NwkUpdateId, 0x24, u8),
    (WatchdogTtl, 0x26, u32),
}
