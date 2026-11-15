use std::{error::Error, fmt, io, string::FromUtf8Error};

#[cfg(all(feature = "tokio-serial", not(feature = "serialport")))]
use tokio_serial as serialport;

/// A result returned by the SF10 library.
pub type Result<T> = std::result::Result<T, VapourtecError>;

/// Connection, validation, or protocol failure while communicating with an SF10.
#[derive(Debug)]
#[non_exhaustive]
pub enum VapourtecError {
    /// Reading, writing, or flushing the transport failed.
    Io(io::Error),
    /// Opening or configuring the native serial port failed.
    #[cfg(any(feature = "serialport", feature = "tokio-serial"))]
    Serial(serialport::Error),
    /// The transport has been disconnected.
    NotConnected,
    /// The pump did not return the expected acknowledgement or status.
    UnexpectedResponse {
        /// Command whose response did not match.
        command: String,
        /// Reply, excluding its terminator and exterior line-feed characters.
        response: String,
    },
    /// A numerical parameter was NaN or infinite.
    InvalidParameter {
        /// Parameter name.
        parameter: &'static str,
        /// Rejected value.
        value: f64,
    },
    /// A response exceeded the bounded frame size without a terminator.
    ResponseTooLong {
        /// Maximum payload length in bytes.
        limit: usize,
    },
    /// A response was not valid UTF-8.
    InvalidResponse(FromUtf8Error),
}

impl fmt::Display for VapourtecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "SF10 transport error: {error}"),
            #[cfg(any(feature = "serialport", feature = "tokio-serial"))]
            Self::Serial(error) => {
                write!(f, "could not open or configure SF10 serial port: {error}")
            }
            Self::NotConnected => f.write_str("SF10 is disconnected"),
            Self::UnexpectedResponse { command, response } => {
                write!(f, "unexpected SF10 response to {command:?}: {response:?}")
            }
            Self::InvalidParameter { parameter, value } => {
                write!(f, "{parameter} must be finite, got {value}")
            }
            Self::ResponseTooLong { limit } => write!(f, "SF10 response exceeded {limit} bytes"),
            Self::InvalidResponse(error) => write!(f, "SF10 response was not valid UTF-8: {error}"),
        }
    }
}

impl Error for VapourtecError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            #[cfg(any(feature = "serialport", feature = "tokio-serial"))]
            Self::Serial(error) => Some(error),
            Self::InvalidResponse(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for VapourtecError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(any(feature = "serialport", feature = "tokio-serial"))]
impl From<serialport::Error> for VapourtecError {
    fn from(error: serialport::Error) -> Self {
        Self::Serial(error)
    }
}

impl From<FromUtf8Error> for VapourtecError {
    fn from(error: FromUtf8Error) -> Self {
        Self::InvalidResponse(error)
    }
}
