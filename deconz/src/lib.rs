mod aps;
mod deconz;
mod errors;
mod parameters;
mod protocol;
mod slip;
mod types;

#[macro_use]
extern crate log;

use std::path::Path;

use tokio_serial::{Serial, SerialPortSettings};

pub use crate::aps::ApsReader;
pub use crate::deconz::Deconz;
pub use crate::errors::{Error, ErrorKind, Result};
pub use crate::parameters::{Parameter, ParameterId, PARAMETERS};
pub use crate::protocol::{CommandId, Request, Response};
pub use crate::slip::SlipError;
pub use crate::types::{
    ApsDataConfirm, ApsDataIndication, ApsDataRequest, ClusterId, Destination, DestinationAddress,
    DeviceState, Endpoint, ExtendedAddress, NetworkState, Platform, ProfileId, SequenceId,
    ShortAddress, SourceAddress, Version,
};

const BAUD: u32 = 38400;

pub fn open_tty<P>(path: P) -> Result<(Deconz, ApsReader)>
where
    P: AsRef<Path>,
{
    let tty = Serial::from_path(
        path,
        &SerialPortSettings {
            baud_rate: BAUD,
            timeout: std::time::Duration::from_secs(60),
            ..Default::default()
        },
    )?;

    let (reader, writer) = tokio::io::split(tty);
    Ok(Deconz::new(reader, writer))
}

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::{Read, Write};

pub trait ReadWire: Sized {
    fn read_wire<R>(r: &mut R) -> Result<Self>
    where
        R: Read;
}

pub trait WriteWire {
    fn wire_len(&self) -> u16;

    fn write_wire<W>(self, w: &mut W) -> Result<()>
    where
        W: Write;
}

impl ReadWire for u8 {
    fn read_wire<R>(r: &mut R) -> Result<Self>
    where
        R: Read,
    {
        Ok(r.read_u8()?)
    }
}

impl WriteWire for u8 {
    fn wire_len(&self) -> u16 {
        1
    }

    fn write_wire<W>(self, w: &mut W) -> Result<()>
    where
        W: Write,
    {
        w.write_u8(self)?;
        Ok(())
    }
}

impl ReadWire for u16 {
    fn read_wire<R>(r: &mut R) -> Result<Self>
    where
        R: Read,
    {
        Ok(r.read_u16::<LittleEndian>()?)
    }
}

impl WriteWire for u16 {
    fn wire_len(&self) -> u16 {
        2
    }

    fn write_wire<W>(self, w: &mut W) -> Result<()>
    where
        W: Write,
    {
        w.write_u16::<LittleEndian>(self)?;
        Ok(())
    }
}

impl ReadWire for u32 {
    fn read_wire<R>(r: &mut R) -> Result<Self>
    where
        R: Read,
    {
        Ok(r.read_u32::<LittleEndian>()?)
    }
}

impl WriteWire for u32 {
    fn wire_len(&self) -> u16 {
        2
    }

    fn write_wire<W>(self, w: &mut W) -> Result<()>
    where
        W: Write,
    {
        w.write_u32::<LittleEndian>(self)?;
        Ok(())
    }
}

impl ReadWire for u64 {
    fn read_wire<R>(r: &mut R) -> Result<Self>
    where
        R: Read,
    {
        Ok(r.read_u64::<LittleEndian>()?)
    }
}

impl WriteWire for u64 {
    fn wire_len(&self) -> u16 {
        8
    }

    fn write_wire<W>(self, w: &mut W) -> Result<()>
    where
        W: Write,
    {
        w.write_u64::<LittleEndian>(self)?;
        Ok(())
    }
}

pub trait ReadWireExt {
    fn read_wire<T>(&mut self) -> Result<T>
    where
        T: ReadWire;
}

impl<R> ReadWireExt for R
where
    R: Read,
{
    fn read_wire<T>(&mut self) -> Result<T>
    where
        T: ReadWire,
    {
        T::read_wire(self)
    }
}

pub trait WriteWireExt {
    fn write_wire<T>(&mut self, value: T) -> Result<()>
    where
        T: WriteWire;
}

impl<W> WriteWireExt for W
where
    W: Write,
{
    fn write_wire<T>(&mut self, value: T) -> Result<()>
    where
        T: WriteWire,
    {
        value.write_wire(self)
    }
}
