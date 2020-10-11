mod errors;
pub mod protocol;

use std::io::Cursor;

use deconz::*;
use tokio::stream::StreamExt;
use tokio::sync::{mpsc, oneshot};
use tophamm_helpers::{awaiting, IncrementingId};

use self::protocol::{ActiveEpRequest, MgmtLqiRequest, SimpleDescRequest};

pub use self::errors::{Error, Result};
pub use self::protocol::{Neighbor, SimpleDescriptor};

type TransactionId = u8;

pub trait Request: WriteWire {
    const CLUSTER_ID: ClusterId;

    type Response: Response;
}

pub trait Response: ReadWire {
    const CLUSTER_ID: ClusterId;
}

type ZdoRequest = (
    TransactionId,
    ApsDataRequest,
    oneshot::Sender<Result<ApsDataIndication>>,
);

type Awaiting = awaiting::Awaiting<TransactionId, ApsDataIndication, Error>;

pub struct Zdo {
    requests: mpsc::Sender<ZdoRequest>,
    transaction_ids: IncrementingId,
}

impl Zdo {
    pub fn new(deconz: Deconz, aps_data_indications: mpsc::Receiver<ApsDataIndication>) -> Self {
        let (requests_tx, requests) = mpsc::channel(1);

        let awaiting = Awaiting::new();
        let rx = Rx {
            awaiting: awaiting.clone(),
            aps_data_indications,
        };
        let tx = Tx {
            deconz,
            awaiting,
            requests,
        };

        tokio::spawn(rx.task());
        tokio::spawn(tx.task());

        Self {
            requests: requests_tx,
            transaction_ids: IncrementingId::new(),
        }
    }

    fn make_frame<R>(&self, id: TransactionId, request: R) -> Result<Vec<u8>>
    where
        R: Request,
        Error: From<R::Error>,
    {
        let mut frame = Vec::new();
        frame.write_wire(id)?;
        frame.write_wire(request)?;
        Ok(frame)
    }

    pub async fn make_request<R>(&self, destination: Destination, request: R) -> Result<R::Response>
    where
        R: Request,
        Error: From<R::Error>,
        Error: From<<R::Response as ReadWire>::Error>,
    {
        let id = self.transaction_ids.next();
        let asdu = self.make_frame(id, request)?;
        let request = ApsDataRequest {
            destination,
            profile_id: ProfileId(0),
            cluster_id: R::CLUSTER_ID,
            source_endpoint: Endpoint(0),
            asdu,
        };

        let (sender, receiver) = oneshot::channel();
        self.requests
            .clone()
            .send((id, request, sender))
            .await
            .unwrap();

        let result = receiver.await?;
        let aps_data_indication = result?;

        // Skip tx_id
        // TODO: assert cluster ID?
        let mut cursor = Cursor::new(&aps_data_indication.asdu[1..]);
        let response = cursor.read_wire()?;

        Ok(response)
    }
}

struct Rx {
    awaiting: Awaiting,
    aps_data_indications: mpsc::Receiver<ApsDataIndication>,
}

impl Rx {
    async fn task(mut self) -> Result<()> {
        while let Some(aps_data_indication) = self.aps_data_indications.next().await {
            let id = aps_data_indication.asdu[0];

            if let Some(Ok(unsolicited)) = self.awaiting.send(&id, Ok(aps_data_indication)) {
                error!("zdo rx: unexpected frame: {:?}", unsolicited);
            }
        }

        Ok(())
    }
}

struct Tx {
    deconz: Deconz,
    awaiting: Awaiting,
    requests: mpsc::Receiver<ZdoRequest>,
}

impl Tx {
    async fn task(mut self) -> Result<()> {
        while let Some((id, request, sender)) = self.requests.next().await {
            let deconz = self.deconz.clone();
            let future = async move { deconz.aps_data_request(request).await };
            tokio::spawn(self.awaiting.clone().register_while(id, sender, future));
        }

        Ok(())
    }
}

// Higher-level helpers. Ideally these would live on an extension trait, but async is not available
// in traits.
impl Zdo {
    pub async fn get_neighbors(&self, destination: Destination) -> Result<Vec<Neighbor>> {
        let mut start_index = 0;
        let mut neighbors = Vec::new();

        loop {
            let resp = self
                .make_request(destination, MgmtLqiRequest { start_index })
                .await?;

            let total = resp.neighbor_table_entries as usize;
            let count = resp.neighbor_table_list.len() as u8;

            neighbors.extend(resp.neighbor_table_list);

            if neighbors.len() >= total {
                return Ok(neighbors);
            }

            start_index += count;
        }
    }

    pub async fn query_endpoints(
        &self,
        addr: ShortAddress,
    ) -> Result<Vec<(Endpoint, SimpleDescriptor)>> {
        let destination = Destination::Nwk(addr, Endpoint(0));
        let resp = self
            .make_request(destination, ActiveEpRequest { addr })
            .await?;

        let mut active_endpoints = Vec::with_capacity(resp.active_endpoints.len());
        for endpoint in resp.active_endpoints {
            let resp = self
                .make_request(destination, SimpleDescRequest { addr, endpoint })
                .await?;
            active_endpoints.push((endpoint, resp.simple_descriptor));
        }

        Ok(active_endpoints)
    }
}
