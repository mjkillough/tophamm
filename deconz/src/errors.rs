use std::fmt::{self, Display};

use crate::{CommandId, ParameterId, SequenceId, SlipError};

#[derive(Debug)]
pub enum ErrorKind {
    DuplicateSequenceId(SequenceId),
    UnsolicitedResponse(SequenceId),
    UnexpectedResponse(CommandId),
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

impl Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::DuplicateSequenceId(sequence_id) => {
                write!(f, "duplicate sequence ID: {}", sequence_id)
            }
            ErrorKind::UnsolicitedResponse(sequence_id) => {
                write!(f, "unsolicited response with sequence ID: {}", sequence_id,)
            }
            ErrorKind::UnexpectedResponse(command_id) => {
                write!(f, "unexpected command ID as response: {}", command_id)
            }
            ErrorKind::UnsupportedCommand(command_id) => {
                write!(f, "unsupported command ID: {}", command_id)
            }
            ErrorKind::UnsupportedParameter(parameter_id) => {
                write!(f, "unsupported parameter ID: {}", parameter_id)
            }
            ErrorKind::InvalidParameter {
                parameter_id,
                inner,
            } => write!(f, "invalid parameter for ID {}: {}", parameter_id, inner),
            ErrorKind::Slip(error) => write!(f, "SLIP error: {}", error),
            ErrorKind::SerialPort(error) => write!(f, "serial port error: {}", error),
            ErrorKind::Io(error) => write!(f, "IO error: {}", error),
            ErrorKind::ChannelError => write!(f, "channel error"),
            ErrorKind::Todo => write!(f, "TODO, oh no"),
        }
    }
}

#[derive(Debug)]
pub struct Error {
    pub kind: ErrorKind,
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "deconz error: {}", self.kind)
    }
}

impl std::error::Error for Error {}

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
