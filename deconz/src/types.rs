use std::fmt::{self, Debug};
use std::io::{Read, Write};

use crate::{Error, ReadWire, ReadWireExt, Result, WriteWire};

pub type SequenceId = u8;

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub struct Endpoint(pub u8);

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub struct ProfileId(pub u16);

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub struct ClusterId(pub u16);

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ShortAddress(pub u16);

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ExtendedAddress(pub u64);

macro_rules! wrapped_primitive {
    ($ident:ident, $repr:expr) => {
        impl ReadWire for $ident {
            type Error = Error;

            fn read_wire<R>(r: &mut R) -> Result<Self>
            where
                R: Read,
            {
                Ok($ident(r.read_wire()?))
            }
        }

        impl WriteWire for $ident {
            type Error = Error;

            fn wire_len(&self) -> u16 {
                self.0.wire_len()
            }

            fn write_wire<W>(self, w: &mut W) -> Result<()>
            where
                W: Write,
            {
                self.0.write_wire(w)?;
                Ok(())
            }
        }

        impl fmt::Debug for $ident {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, $repr, self.0)
            }
        }
    };
}

wrapped_primitive!(Endpoint, "{:#04x}");
wrapped_primitive!(ProfileId, "{:#06x}");
wrapped_primitive!(ClusterId, "{:#06x}");
wrapped_primitive!(ShortAddress, "{:#06x}");
wrapped_primitive!(ExtendedAddress, "{:#010x}");

#[derive(Copy, Clone, Debug)]
pub enum Platform {
    Avr,
    Arm,
    Unknown(u8),
}

#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Version {
    pub major: u8,
    pub minor: u8,
}

#[derive(Copy, Clone, Debug)]
pub enum NetworkState {
    Offline,
    Joining,
    Connected,
    Leaving,
}

#[derive(Copy, Clone, Debug)]
pub struct DeviceState {
    pub network_state: NetworkState,
    pub data_confirm: bool,
    pub data_indication: bool,
    pub data_request_free_slots: bool,
    pub configuration_changed: bool,
}

impl Default for DeviceState {
    fn default() -> Self {
        Self {
            network_state: NetworkState::Offline,
            data_confirm: false,
            data_indication: false,
            data_request_free_slots: false,
            configuration_changed: false,
        }
    }
}

#[derive(Debug)]
pub enum DestinationAddress {
    Group(ShortAddress),
    Nwk(ShortAddress),
    Ieee(ExtendedAddress),
}

pub struct SourceAddress {
    pub short: ShortAddress,
    pub extended: ExtendedAddress,
}

impl Debug for SourceAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SourceAddress({:?}, {:?})", self.short, self.extended)
    }
}

#[derive(Debug)]
pub struct ApsDataIndication {
    pub destination_address: DestinationAddress,
    pub destination_endpoint: Endpoint,
    pub source_address: SourceAddress,
    pub source_endpoint: Endpoint,
    pub profile_id: ProfileId,
    pub cluster_id: ClusterId,
    pub asdu: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub enum Destination {
    Group(ShortAddress),
    Nwk(ShortAddress, Endpoint),
    Ieee(ExtendedAddress, Endpoint),
}

#[derive(Debug)]
pub struct ApsDataRequest {
    pub destination: Destination,
    pub profile_id: ProfileId,
    pub cluster_id: ClusterId,
    pub source_endpoint: Endpoint,
    pub asdu: Vec<u8>,
}

#[derive(Debug)]
pub struct ApsDataConfirm {
    pub destination: Destination,
    pub source_endpoint: Endpoint,
    pub status: u8,
}
