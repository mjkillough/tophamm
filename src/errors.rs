use crate::{ParameterId, SequenceId, SlipError};

#[derive(Debug)]
pub enum ErrorKind {
    DuplicateSequenceId(SequenceId),
    UnsupportedCommand(u8),
    UnsupportedParameter(u8),
    InvalidParameter {
        parameter_id: ParameterId,
        inner: Box<Error>,
    },
    Slip(SlipError),
    SerialPort(tokio_serial::Error),
    Io(std::io::Error),
    ChannelError,
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

impl From<ErrorKind> for Error {
    fn from(kind: ErrorKind) -> Self {
        Self { kind }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
