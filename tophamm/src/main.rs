#[macro_use]
extern crate log;

mod zdo;

use deconz::{Destination, Endpoint, ShortAddress};
use tokio::stream::StreamExt;
use tokio::sync::mpsc;

use crate::zdo::{Result, Zdo};

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

    for neighbor in zdo
        .get_neighbors(Destination::Nwk(ShortAddress(0x0), Endpoint(0)))
        .await?
    {
        debug!("querying neighbor {:?}", neighbor.network_address);

        let endpoints = zdo.query_endpoints(neighbor.network_address).await?;
        info!(
            "neighbor = {:?}, endpoints = {:?}",
            neighbor.network_address, endpoints
        );
    }

    // dbg!(fut1.await?);
    // dbg!(fut3.await?);

    loop {}
}
