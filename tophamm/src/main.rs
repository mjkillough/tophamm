// #[macro_use]
// extern crate log;

use anyhow::Result;
use deconz::{ApsDataRequest, Destination};
use tokio::stream::StreamExt;

#[tokio::main]
async fn main() -> Result<()> {
    pretty_env_logger::init();

    let args = std::env::args().collect::<Vec<_>>();
    let path = &args[1];

    let (deconz, aps_reader) = deconz::open_tty(path)?;

    // let fut1 = deconz.version();
    let fut2 = deconz.device_state();
    let fut3 = deconz.aps_data_request(ApsDataRequest {
        destination: Destination::Nwk(345, 0),
        profile_id: 0,
        cluster_id: 0x5,
        source_endpoint: 0,
        asdu: vec![0x0, 0x59, 0x1],
    });

    tokio::spawn(async move {
        let mut aps_reader = aps_reader;
        while let Some(aps_data_indication) = aps_reader.next().await {
            dbg!(aps_data_indication);
        }
    });

    dbg!(fut2.await?);
    // dbg!(fut1.await?);
    dbg!(fut3.await?);

    loop {}
}
