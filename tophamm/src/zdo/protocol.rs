use std::io::{Read, Write};

use deconz::{
    ClusterId, Endpoint, ExtendedAddress, ProfileId, ReadWire, ReadWireExt, ShortAddress,
    WriteWire, WriteWireExt,
};

use super::{Error, Request, Response, Result};

#[derive(Debug)]
pub struct SimpleDescRequest {
    pub addr: ShortAddress,
    pub endpoint: Endpoint,
}

impl Request for SimpleDescRequest {
    const CLUSTER_ID: ClusterId = ClusterId(0x0004);

    type Response = SimpleDescResponse;
}

impl WriteWire for SimpleDescRequest {
    type Error = Error;

    fn wire_len(&self) -> u16 {
        3
    }

    fn write_wire<W>(self, w: &mut W) -> Result<()>
    where
        W: Write,
    {
        w.write_wire(self.addr)?;
        w.write_wire(self.endpoint)?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct SimpleDescResponse {
    pub status: u8,
    pub addr: ShortAddress,
    pub simple_descriptor: SimpleDescriptor,
}

impl Response for SimpleDescResponse {
    const CLUSTER_ID: ClusterId = ClusterId(0x8004);
}

impl ReadWire for SimpleDescResponse {
    type Error = Error;

    fn read_wire<R>(r: &mut R) -> Result<Self>
    where
        R: Read,
    {
        let status = r.read_wire()?;
        let addr = r.read_wire()?;
        let _len: u8 = r.read_wire()?;

        let endpoint = r.read_wire()?;
        let profile = r.read_wire()?;
        let device_identifier = r.read_wire()?;
        let device_version = r.read_wire()?;

        let input_count: u8 = r.read_wire()?;
        let mut input_clusters = Vec::with_capacity(usize::from(input_count));
        for _ in 0..input_count {
            input_clusters.push(r.read_wire()?);
        }

        let output_count: u8 = r.read_wire()?;
        let mut output_clusters = Vec::with_capacity(usize::from(output_count));
        for _ in 0..output_count {
            output_clusters.push(r.read_wire()?);
        }

        let simple_descriptor = SimpleDescriptor {
            endpoint,
            profile,
            device_identifier,
            device_version,
            input_clusters,
            output_clusters,
        };

        Ok(SimpleDescResponse {
            status,
            addr,
            simple_descriptor,
        })
    }
}

#[derive(Debug)]
pub struct SimpleDescriptor {
    pub endpoint: Endpoint,
    pub profile: ProfileId,
    pub device_identifier: u16,
    pub device_version: u8, // 4 bits
    pub input_clusters: Vec<ClusterId>,
    pub output_clusters: Vec<ClusterId>,
}

#[derive(Debug)]
pub struct ActiveEpRequest {
    pub addr: ShortAddress,
}

impl Request for ActiveEpRequest {
    const CLUSTER_ID: ClusterId = ClusterId(0x0005);

    type Response = ActiveEpResponse;
}

impl WriteWire for ActiveEpRequest {
    type Error = Error;

    fn wire_len(&self) -> u16 {
        2
    }

    fn write_wire<W>(self, w: &mut W) -> Result<()>
    where
        W: Write,
    {
        w.write_wire(self.addr)?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct ActiveEpResponse {
    pub status: u8,
    pub addr: ShortAddress,
    pub active_endpoints: Vec<Endpoint>,
}

impl Response for ActiveEpResponse {
    const CLUSTER_ID: ClusterId = ClusterId(0x8005);
}

impl ReadWire for ActiveEpResponse {
    type Error = Error;

    fn read_wire<R>(r: &mut R) -> Result<Self>
    where
        R: Read,
    {
        let status = r.read_wire()?;
        let addr = r.read_wire()?;

        let count: u8 = r.read_wire()?;
        let mut active_endpoints = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            active_endpoints.push(r.read_wire()?);
        }

        Ok(ActiveEpResponse {
            status,
            addr,
            active_endpoints,
        })
    }
}

#[derive(Debug)]
pub struct MgmtLqiRequest {
    pub start_index: u8,
}

impl Request for MgmtLqiRequest {
    const CLUSTER_ID: ClusterId = ClusterId(0x0031);

    type Response = MgmtLqiResponse;
}

impl WriteWire for MgmtLqiRequest {
    type Error = Error;

    fn wire_len(&self) -> u16 {
        1
    }

    fn write_wire<W>(self, w: &mut W) -> Result<()>
    where
        W: Write,
    {
        w.write_wire(self.start_index)?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct MgmtLqiResponse {
    pub status: u8,
    pub neighbor_table_entries: u8,
    pub start_index: u8,
    pub neighbor_table_list: Vec<Neighbor>,
}

impl Response for MgmtLqiResponse {
    const CLUSTER_ID: ClusterId = ClusterId(0x8031);
}

impl ReadWire for MgmtLqiResponse {
    type Error = Error;

    fn read_wire<R>(r: &mut R) -> Result<Self>
    where
        R: Read,
    {
        let status = r.read_wire()?;
        let neighbor_table_entries = r.read_wire()?;
        let start_index = r.read_wire()?;

        let count: u8 = r.read_wire()?;
        let mut neighbor_table_list = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            let extended_pan_id = r.read_wire()?;
            let extended_address = r.read_wire()?;
            let network_address = r.read_wire()?;

            let byte: u8 = r.read_wire()?;
            let device_type = match byte & 0b11 {
                0x0 => DeviceType::Coordinator,
                0x1 => DeviceType::Router,
                0x2 => DeviceType::EndDevice,
                0x3 => DeviceType::Unknown,
                _ => unreachable!("bitfield"),
            };
            let rx_on_while_idle = match (byte >> 2) & 0b11 {
                0x0 => RxOnWhileIdle::Off,
                0x1 => RxOnWhileIdle::On,
                0x2 => RxOnWhileIdle::Unknown,
                0x3 => RxOnWhileIdle::Unknown, // better than panicking
                _ => unreachable!("bitfield"),
            };
            let relationship = match (byte >> 4) & 0b111 {
                0x0 => NeighborRelationship::Parent,
                0x1 => NeighborRelationship::Child,
                0x2 => NeighborRelationship::Sibling,
                0x3 => NeighborRelationship::None,
                0x4 => NeighborRelationship::PreviousChild,
                _ => unreachable!("bitfield"),
            };

            let byte: u8 = r.read_wire()?;
            let permit_joining = match byte & 0b11 {
                0x0 => PermitJoining::Accepting,
                0x1 => PermitJoining::NotAccepting,
                0x2 => PermitJoining::Unknown,
                0x3 => PermitJoining::Unknown, // better than panicking
                _ => unreachable!("bitfield"),
            };

            let depth = r.read_wire()?;
            let link_quality_index = r.read_wire()?;

            neighbor_table_list.push(Neighbor {
                extended_pan_id,
                extended_address,
                network_address,
                device_type,
                rx_on_while_idle,
                relationship,
                permit_joining,
                depth,
                link_quality_index,
            });
        }

        Ok(MgmtLqiResponse {
            status,
            neighbor_table_entries,
            start_index,
            neighbor_table_list,
        })
    }
}

#[derive(Debug)]
pub enum DeviceType {
    Coordinator,
    Router,
    EndDevice,
    Unknown,
}

#[derive(Debug)]
pub enum RxOnWhileIdle {
    Off,
    On,
    Unknown,
}

#[derive(Debug)]
pub enum NeighborRelationship {
    Parent,
    Child,
    Sibling,
    None,
    PreviousChild,
}

#[derive(Debug)]
pub enum PermitJoining {
    Accepting,
    NotAccepting,
    Unknown,
}

#[derive(Debug)]
pub struct Neighbor {
    pub extended_pan_id: u64,
    pub extended_address: ExtendedAddress,
    pub network_address: ShortAddress,
    pub device_type: DeviceType,
    pub rx_on_while_idle: RxOnWhileIdle,
    pub relationship: NeighborRelationship,
    pub permit_joining: PermitJoining,
    pub depth: u8,
    pub link_quality_index: u8,
}
