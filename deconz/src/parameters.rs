use std::convert::TryFrom;
use std::fmt::{self, Display};
use std::io::{Cursor, Write};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::{Error, ErrorKind, Result};

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

            pub fn len(&self) -> u16 {
                match self {
                    $(Parameter::$param(_) => <$ty as ConvertParameter>::len()),+
                }
            }

            pub fn write<W>(&self, buffer: W) -> Result<()>
            where
                W: Write,
            {
                match self {
                    $(Parameter::$param(value) => ConvertParameter::write(value, buffer)),+
                }
            }
        }

        impl ParameterId {
            pub fn read_parameter(&self, buffer: &[u8]) -> Result<Parameter> {
                match self {
                    $(
                        ParameterId::$param => {
                            let param = ConvertParameter::read(buffer)
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

        impl TryFrom<u8> for ParameterId {
            type Error = Error;

            fn try_from(byte: u8) -> Result<Self> {
                match byte {
                    $($id => Ok(ParameterId::$param),)+
                    _ => Err(Error { kind: ErrorKind::UnsupportedParameter(byte) }),
                }
            }
        }

        impl From<ParameterId> for u8 {
            fn from(id: ParameterId) -> u8 {
                match id {
                    $(ParameterId::$param => $id,)+
                }
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

trait ConvertParameter: Sized {
    fn len() -> u16;
    fn read(buffer: &[u8]) -> Result<Self>;
    fn write<W>(&self, buffer: W) -> Result<()>
    where
        W: Write;
}

impl ConvertParameter for u8 {
    fn len() -> u16 {
        1
    }

    fn read(buffer: &[u8]) -> Result<Self> {
        Ok(Cursor::new(buffer).read_u8()?)
    }

    fn write<W>(&self, mut buffer: W) -> Result<()>
    where
        W: Write,
    {
        buffer.write_u8(*self)?;
        Ok(())
    }
}

impl ConvertParameter for u16 {
    fn len() -> u16 {
        2
    }

    fn read(buffer: &[u8]) -> Result<Self> {
        Ok(Cursor::new(buffer).read_u16::<LittleEndian>()?)
    }

    fn write<W>(&self, mut buffer: W) -> Result<()>
    where
        W: Write,
    {
        buffer.write_u16::<LittleEndian>(*self)?;
        Ok(())
    }
}

impl ConvertParameter for u32 {
    fn len() -> u16 {
        4
    }

    fn read(buffer: &[u8]) -> Result<Self> {
        Ok(Cursor::new(buffer).read_u32::<LittleEndian>()?)
    }

    fn write<W>(&self, mut buffer: W) -> Result<()>
    where
        W: Write,
    {
        buffer.write_u32::<LittleEndian>(*self)?;
        Ok(())
    }
}

impl ConvertParameter for u64 {
    fn len() -> u16 {
        8
    }

    fn read(buffer: &[u8]) -> Result<Self> {
        Ok(Cursor::new(buffer).read_u64::<LittleEndian>()?)
    }

    fn write<W>(&self, mut buffer: W) -> Result<()>
    where
        W: Write,
    {
        buffer.write_u64::<LittleEndian>(*self)?;
        Ok(())
    }
}
