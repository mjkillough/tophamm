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
