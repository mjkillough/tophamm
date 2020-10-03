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

#[derive(Debug)]
pub enum DestinationAddress {
    Group(ShortAddress),
    Nwk(ShortAddress),
    Ieee(ExtendedAddress),
}

#[derive(Debug)]
pub struct SourceAddress {
    pub short: ShortAddress,
    pub extended: ExtendedAddress,
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

#[derive(Debug)]
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
