use crate::ParameterId;
use crate::SlipError;

#[derive(Debug)]
pub enum ErrorKind {
    UnsupportedCommand(u8),
    UnsupportedParameter(u8),
    InvalidParameter {
        parameter_id: ParameterId,
        inner: Box<Error>,
    },
    Slip(SlipError),
    SerialPort(tokio_serial::Error),
    Io(std::io::Error),
    Todo,
}

#[derive(Debug)]
pub struct Error {
    pub kind: ErrorKind,
}

impl From<std::io::Error> for Error {
    fn from(other: std::io::Error) -> Self {
        Error {
            kind: ErrorKind::Io(other),
        }
    }
}

impl From<tokio_serial::Error> for Error {
    fn from(other: tokio_serial::Error) -> Self {
        Error {
            kind: ErrorKind::SerialPort(other),
        }
    }
}

impl From<SlipError> for Error {
    fn from(other: SlipError) -> Self {
        Error {
            kind: ErrorKind::Slip(other),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
