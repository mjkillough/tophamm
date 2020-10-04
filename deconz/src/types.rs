use std::fmt::{self, Debug};

pub type SequenceId = u8;
pub type Endpoint = u8;
pub type ProfileId = u16;
pub type ClusterId = u16;
pub type ShortAddress = u16;
pub type ExtendedAddress = u64;

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

pub enum DestinationAddress {
    Group(ShortAddress),
    Nwk(ShortAddress),
    Ieee(ExtendedAddress),
}

impl Debug for DestinationAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DestinationAddress::Group(addr) => write!(f, "Group({:#04x})", addr),
            DestinationAddress::Nwk(addr) => write!(f, "Nwk({:#04x})", addr),
            DestinationAddress::Ieee(addr) => write!(f, "Ieee({:#016x})", addr),
        }
    }
}

pub struct SourceAddress {
    pub short: ShortAddress,
    pub extended: ExtendedAddress,
}

impl Debug for SourceAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SourceAddress({:#04x}, {:#016x})",
            self.short, self.extended
        )
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

pub enum Destination {
    Group(ShortAddress),
    Nwk(ShortAddress, Endpoint),
    Ieee(ExtendedAddress, Endpoint),
}

impl Debug for Destination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Destination::Group(addr) => write!(f, "Group({:#04x})", addr),
            Destination::Nwk(addr, endpoint) => write!(f, "Nwk({:#04x}, {:#02x})", addr, endpoint),
            Destination::Ieee(addr, endpoint) => {
                write!(f, "Ieee({:#016x}, {:#02x})", addr, endpoint)
            }
        }
    }
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
