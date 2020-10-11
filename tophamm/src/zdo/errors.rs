use std::fmt::{self, Display};
use std::io;

use tokio::sync::oneshot;

#[derive(Debug)]
pub enum ErrorKind {
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
pub struct Error {
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

pub type Result<T> = std::result::Result<T, Error>;
