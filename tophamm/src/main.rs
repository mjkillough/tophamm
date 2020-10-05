#[macro_use]
extern crate log;

use std::collections::HashMap;
use std::fmt::{self, Display};
use std::io::{self, Cursor, Read, Write};
use std::sync::{Arc, Mutex};

use deconz::*;
use tokio::stream::StreamExt;
use tokio::sync::{mpsc, oneshot};

type TransactionId = u8;

#[derive(Debug)]
enum ErrorKind {
    Deconz(deconz::Error),
    Io(io::Error),
    ChannelError,
}

impl Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::Deconz(error) => write!(f, "deconz: {}", error),
            ErrorKind::Io(error) => write!(f, "io: {}", error),
            ErrorKind::ChannelError => write!(f, "channel error"),
        }
    }
}

#[derive(Debug)]
struct Error {
    kind: ErrorKind,
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind)
    }
}

impl std::error::Error for Error {}

impl From<deconz::Error> for Error {
    fn from(other: deconz::Error) -> Self {
        Error {
            kind: ErrorKind::Deconz(other),
        }
    }
}

impl From<io::Error> for Error {
    fn from(other: io::Error) -> Self {
        Error {
            kind: ErrorKind::Io(other),
        }
    }
}

impl From<oneshot::error::RecvError> for Error {
    fn from(_: oneshot::error::RecvError) -> Error {
        Error {
            kind: ErrorKind::ChannelError,
        }
    }
}

type Result<T> = std::result::Result<T, Error>;

trait Request: WriteWire {
    const CLUSTER_ID: ClusterId;

    type Response: Response;
}

trait Response: ReadWire {
    const CLUSTER_ID: ClusterId;
}

struct Zdo {
    commands: mpsc::Sender<(ApsDataRequest, oneshot::Sender<Result<ApsDataIndication>>)>,
}

impl Zdo {
    fn new(deconz: Deconz, aps_data_indications: mpsc::Receiver<ApsDataIndication>) -> Self {
        let (commands_tx, commands_rx) = mpsc::channel(1);

        let shared = Arc::new(Shared {
            awaiting: Default::default(),
        });

        let rx = Rx {
            shared: shared.clone(),
            aps_data_indications,
        };
        let tx = Tx {
            shared,
            deconz,
            commands: commands_rx,
            tx_id: 0,
        };

        tokio::spawn(rx.task());
        tokio::spawn(tx.task());

        Self {
            commands: commands_tx,
        }
    }

    fn make_frame<R>(&self, request: R) -> Result<Vec<u8>>
    where
        R: Request,
        Error: From<R::Error>,
    {
        let mut frame = Vec::new();
        // We write a dummy transaction ID here. The Tx task will fill it in.
        frame.write_wire(0 as u8)?;
        frame.write_wire(request)?;
        Ok(frame)
    }

    async fn make_request<R>(&self, destination: Destination, request: R) -> Result<R::Response>
    where
        R: Request,
        Error: From<R::Error>,
        Error: From<<R::Response as ReadWire>::Error>,
    {
        let asdu = self.make_frame(request)?;
        let request = ApsDataRequest {
            destination,
            profile_id: ProfileId(0),
            cluster_id: R::CLUSTER_ID,
            source_endpoint: Endpoint(0),
            asdu,
        };

        let (sender, receiver) = oneshot::channel();
        self.commands.clone().send((request, sender)).await.unwrap();

        let result = receiver.await?;
        let aps_data_indication = result?;

        // Skip tx_id
        // TODO: assert cluster ID?
        let mut cursor = Cursor::new(&aps_data_indication.asdu[1..]);
        let response = cursor.read_wire()?;

        Ok(response)
    }
}

struct Shared {
    awaiting: Mutex<HashMap<TransactionId, oneshot::Sender<Result<ApsDataIndication>>>>,
}

struct Rx {
    shared: Arc<Shared>,
    aps_data_indications: mpsc::Receiver<ApsDataIndication>,
}

impl Rx {
    async fn task(mut self) -> Result<()> {
        while let Some(aps_data_indication) = self.aps_data_indications.next().await {
            let tx_id = aps_data_indication.asdu[0];

            match self.shared.awaiting.lock().unwrap().remove(&tx_id) {
                Some(sender) => {
                    sender.send(Ok(aps_data_indication)).unwrap();
                }
                None => {
                    error!("zdo rx: unexpected frame: {:?}", aps_data_indication);
                }
            }
        }

        Ok(())
    }
}

struct Tx {
    shared: Arc<Shared>,
    deconz: Deconz,
    commands: mpsc::Receiver<(ApsDataRequest, oneshot::Sender<Result<ApsDataIndication>>)>,
    tx_id: u8,
}

impl Tx {
    async fn task(mut self) -> Result<()> {
        while let Some((mut request, sender)) = self.commands.next().await {
            let tx_id = self.tx_id();
            let shared = self.shared.clone();
            let deconz = self.deconz.clone();
            tokio::spawn(async move {
                shared.awaiting.lock().unwrap().insert(tx_id, sender);

                request.asdu[0] = tx_id;

                debug!("aps_data_request: {:?}", request);

                if let Err(error) = deconz.aps_data_request(request).await {
                    error!("zdo tx: {}", error);
                    shared.awaiting.lock().unwrap().remove(&tx_id);
                }
            });
        }

        Ok(())
    }

    fn tx_id(&mut self) -> TransactionId {
        let old = self.tx_id;
        self.tx_id += 1;
        old
    }
}

#[derive(Debug)]
struct SimpleDescRequest {
    addr: ShortAddress,
    endpoint: Endpoint,
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
struct SimpleDescResponse {
    status: u8,
    addr: ShortAddress,
    simple_descriptor: SimpleDescriptor,
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

// pg 96
#[derive(Debug)]
struct SimpleDescriptor {
    endpoint: Endpoint,
    profile: ProfileId,
    device_identifier: u16,
    device_version: u8, // 4 bits
    input_clusters: Vec<ClusterId>,
    output_clusters: Vec<ClusterId>,
}

#[derive(Debug)]
struct ActiveEpRequest {
    addr: ShortAddress,
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
struct ActiveEpResponse {
    status: u8,
    addr: ShortAddress,
    active_endpoints: Vec<Endpoint>,
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
struct MgmtLqiRequest {
    start_index: u8,
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
struct MgmtLqiResponse {
    status: u8,
    neighbor_table_entries: u8,
    start_index: u8,
    neighbor_table_list: Vec<Neighbor>,
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
enum DeviceType {
    Coordinator,
    Router,
    EndDevice,
    Unknown,
}

#[derive(Debug)]
enum RxOnWhileIdle {
    Off,
    On,
    Unknown,
}

#[derive(Debug)]
enum NeighborRelationship {
    Parent,
    Child,
    Sibling,
    None,
    PreviousChild,
}

#[derive(Debug)]
enum PermitJoining {
    Accepting,
    NotAccepting,
    Unknown,
}

#[derive(Debug)]
struct Neighbor {
    extended_pan_id: u64,
    extended_address: ExtendedAddress,
    network_address: ShortAddress,
    device_type: DeviceType,
    rx_on_while_idle: RxOnWhileIdle,
    relationship: NeighborRelationship,
    permit_joining: PermitJoining,
    depth: u8,
    link_quality_index: u8,
}

async fn get_neighbors(zdo: &Zdo, destination: Destination) -> Result<Vec<Neighbor>> {
    let mut start_index = 0;
    let mut neighbors = Vec::new();

    loop {
        let resp = zdo
            .make_request(destination, MgmtLqiRequest { start_index })
            .await?;
        dbg!(&resp);
        let total = resp.neighbor_table_entries as usize;
        let count = resp.neighbor_table_list.len() as u8;

        neighbors.extend(resp.neighbor_table_list);

        if neighbors.len() >= total {
            return Ok(neighbors);
        }

        start_index += count;
    }
}

async fn query_endpoints(
    zdo: &Zdo,
    addr: ShortAddress,
) -> Result<Vec<(Endpoint, SimpleDescriptor)>> {
    let destination = Destination::Nwk(addr, Endpoint(0));
    let resp = zdo
        .make_request(destination, ActiveEpRequest { addr })
        .await?;

    let mut active_endpoints = Vec::with_capacity(resp.active_endpoints.len());
    for endpoint in resp.active_endpoints {
        let resp = zdo
            .make_request(destination, SimpleDescRequest { addr, endpoint })
            .await?;
        active_endpoints.push((endpoint, resp.simple_descriptor));
    }

    Ok(active_endpoints)
}

#[tokio::main]
async fn main() -> Result<()> {
    pretty_env_logger::init();

    let args = std::env::args().collect::<Vec<_>>();
    let path = &args[1];

    let (deconz, aps_reader) = deconz::open_tty(path)?;

    // let fut1 = deconz.version();
    let fut2 = deconz.device_state();

    let (zdo_tx, zdo_rx) = mpsc::channel(1);
    let zdo = Zdo::new(deconz.clone(), zdo_rx);

    tokio::spawn(async move {
        let mut aps_reader = aps_reader;
        let mut zdo_tx = zdo_tx;

        while let Some(aps_data_indication) = aps_reader.next().await {
            if aps_data_indication.destination_endpoint == Endpoint(0) {
                debug!("zdo frame: {:?}", aps_data_indication);
                zdo_tx.send(aps_data_indication).await.unwrap()
            } else {
                debug!("other frame: {:?}", aps_data_indication);
            }
        }
    });

    // let fut3 = deconz.aps_data_request(ApsDataRequest {
    //     destination: Destination::Nwk(345, 0),
    //     profile_id: 0,
    //     cluster_id: 0x5,
    //     source_endpoint: 0,
    //     asdu: vec![0x0, 0x59, 0x1],
    // });

    dbg!(fut2.await?);

    for neighbor in get_neighbors(&zdo, Destination::Nwk(ShortAddress(0x0), Endpoint(0))).await? {
        let endpoints = query_endpoints(&zdo, neighbor.network_address).await?;
        info!(
            "neighbor = {:?}, endpoints = {:?}",
            neighbor.network_address, endpoints
        );
    }

    // dbg!(fut1.await?);
    // dbg!(fut3.await?);

    loop {}
}
