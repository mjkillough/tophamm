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

use tokio::stream::StreamExt;
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

#[tokio::main]
async fn main() -> Result<()> {
    pretty_env_logger::init();

    let args = std::env::args().collect::<Vec<_>>();
    let path = &args[1];

    let (deconz, aps_reader) = open_tty(path)?;

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
