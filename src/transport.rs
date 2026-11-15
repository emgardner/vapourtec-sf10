//! Optional helpers for creating configured serial transports.
//!
//! These helpers open transports only. Pass the returned instance to
//! [`crate::Sf10::from_transport`] or, with the `async` feature,
//! `AsyncSf10::from_transport` to perform the pump handshake.

/// Concrete synchronous serial transport on Unix platforms.
#[cfg(all(feature = "serialport", unix))]
pub type NativeSerialPort = serialport::TTYPort;

/// Concrete synchronous serial transport on Windows.
#[cfg(all(feature = "serialport", windows))]
pub type NativeSerialPort = serialport::COMPort;

/// Open a concrete synchronous serial port configured for an SF10.
///
/// Uses 9600 baud, 8 data bits, no parity, one stop bit, and no flow control.
/// Sets the supplied read/write timeout and clears stale input before returning.
/// Opening the port does not enable remote control or send any pump commands.
///
/// ```no_run
/// use std::time::Duration;
/// use vapourtec::{open_serial_port, Sf10};
///
/// # fn main() -> vapourtec::Result<()> {
/// let transport = open_serial_port("COM6", Duration::from_secs(1))?;
/// let mut pump = Sf10::from_transport(transport)?;
/// pump.set_flow_rate(1.0)?;
/// # Ok(())
/// # }
/// ```
#[cfg(all(feature = "serialport", any(unix, windows)))]
pub fn open_serial_port(
    device_port: impl AsRef<str>,
    timeout: std::time::Duration,
) -> crate::Result<NativeSerialPort> {
    use serialport::{ClearBuffer, DataBits, FlowControl, Parity, SerialPort, StopBits};

    let transport = serialport::new(device_port.as_ref(), 9_600)
        .data_bits(DataBits::Eight)
        .parity(Parity::None)
        .stop_bits(StopBits::One)
        .flow_control(FlowControl::None)
        .timeout(timeout)
        .open_native()?;
    transport.clear(ClearBuffer::Input)?;
    Ok(transport)
}

/// Open an asynchronous serial stream configured for an SF10.
///
/// Uses 9600 baud, 8 data bits, no parity, one stop bit, and no flow control,
/// and clears stale input before returning. Opening the port does not enable
/// remote control or send any pump commands.
///
/// Call this synchronous constructor inside a Tokio runtime with its I/O driver
/// enabled. Command deadlines are configured on `AsyncSf10`, rather than by a
/// blocking serial-port timeout.
///
/// ```no_run
/// use vapourtec::{open_tokio_serial_port, AsyncSf10};
///
/// # async fn connect() -> vapourtec::Result<()> {
/// let transport = open_tokio_serial_port("COM6")?;
/// let mut pump = AsyncSf10::from_transport(transport).await?;
/// pump.set_flow_rate(1.0).await?;
/// # Ok(())
/// # }
/// ```
#[cfg(feature = "tokio-serial")]
pub fn open_tokio_serial_port(
    device_port: impl AsRef<str>,
) -> crate::Result<tokio_serial::SerialStream> {
    use tokio_serial::{
        ClearBuffer, DataBits, FlowControl, Parity, SerialPort, SerialPortBuilderExt, StopBits,
    };

    let transport = tokio_serial::new(device_port.as_ref(), 9_600)
        .data_bits(DataBits::Eight)
        .parity(Parity::None)
        .stop_bits(StopBits::One)
        .flow_control(FlowControl::None)
        .open_native_async()?;
    transport.clear(ClearBuffer::Input)?;
    Ok(transport)
}
