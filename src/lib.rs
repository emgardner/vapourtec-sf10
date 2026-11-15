//! SF10 pump clients over application-provided I/O transports.
//!
//! [`Sf10<T>`] accepts any blocking [`std::io::Read`] + [`std::io::Write`]
//! transport. The `async` feature adds `AsyncSf10<T>` over Tokio's `AsyncRead` +
//! `AsyncWrite` traits. Neither client requires a particular serial driver.
//!
//! Optional `serialport` and `tokio-serial` features provide helpers that open
//! concrete serial transports with the SF10's 9600 baud, 8N1 settings.
//!
//! ```no_run
//! # #[cfg(all(feature = "serialport", any(unix, windows)))]
//! # fn main() -> vapourtec::Result<()> {
//! use std::time::Duration;
//! use vapourtec::{open_serial_port, Sf10};
//!
//! let port = open_serial_port("COM6", Duration::from_secs(1))?;
//! let mut pump = Sf10::from_transport(port)?;
//! pump.set_flow_rate(1.0)?;
//! pump.start()?;
//! pump.stop()?;
//! # Ok(())
//! # }
//! # #[cfg(not(all(feature = "serialport", any(unix, windows))))]
//! # fn main() {}
//! ```
//!
//! ```no_run
//! # #[cfg(feature = "tokio-serial")]
//! # async fn example() -> vapourtec::Result<()> {
//! use vapourtec::{open_tokio_serial_port, AsyncSf10};
//!
//! let port = open_tokio_serial_port("COM6")?;
//! let mut pump = AsyncSf10::from_transport(port).await?;
//! pump.set_flow_rate(1.0).await?;
//! pump.start().await?;
//! pump.stop().await?;
//! # Ok(())
//! # }
//! ```
//!
//! State records acknowledged requests, not physical readback. Disconnecting or
//! dropping a client releases its transport without sending `STOP`.

#![forbid(unsafe_code)]

#[cfg(feature = "async")]
mod async_sf10;
mod error;
mod options;
mod protocol;
mod sf10;
mod state;
#[cfg(any(feature = "serialport", feature = "tokio-serial"))]
mod transport;

#[cfg(feature = "async")]
pub use async_sf10::AsyncSf10;
pub use error::{Result, VapourtecError};
pub use options::ConnectionOptions;
pub use sf10::Sf10;
pub use state::{Mode, PumpState, Valve};
#[cfg(feature = "tokio-serial")]
pub use transport::open_tokio_serial_port;
#[cfg(all(feature = "serialport", any(unix, windows)))]
pub use transport::{open_serial_port, NativeSerialPort};
