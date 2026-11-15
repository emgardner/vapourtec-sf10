use std::io;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::protocol::{
    decode_response, number, rounded_flow, validate_finite, MAX_RESPONSE_BYTES, REMOTE_ENABLED,
};
use crate::{ConnectionOptions, Mode, PumpState, Result, Valve, VapourtecError};

/// An asynchronous SF10 connection over a caller-supplied transport.
///
/// Accepts any Tokio [`AsyncRead`] + [`AsyncWrite`] + [`Unpin`] transport without
/// requiring boxing, `Send`, or a concrete serial-port implementation. The
/// caller configures and opens the transport before transferring ownership.
///
/// Every command has the whole-exchange deadline specified by
/// [`ConnectionOptions::timeout`], covering writes, flushing, and the complete
/// reply. Commands require a Tokio runtime with its time driver enabled.
///
/// An I/O error, malformed reply, timeout, or cancellation of an in-flight
/// command releases the transport and clears cached state, preventing a late
/// acknowledgement from being attributed to a later command. Dropping this
/// connection never sends `STOP` or disables remote control.
pub struct AsyncSf10<T> {
    transport: Option<T>,
    state: PumpState,
    firmware_version: String,
    options: ConnectionOptions,
}

impl<T> AsyncSf10<T> {
    /// Return the last acknowledged requests. Unknown settings are `None`.
    pub fn state(&self) -> &PumpState {
        &self.state
    }

    /// Return the unparsed firmware reply obtained during initialization.
    ///
    /// Firmware 1.8 is the original implementation's documented prerequisite;
    /// the library does not infer compatibility from the format of this reply.
    pub fn firmware_version(&self) -> &str {
        &self.firmware_version
    }

    /// Whether this instance still owns a transport.
    ///
    /// This does not actively probe whether a physical device is connected.
    pub fn is_connected(&self) -> bool {
        self.transport.is_some()
    }

    /// Release the transport and clear cached settings without sending commands.
    ///
    /// Repeated calls are harmless. Call [`Self::stop`] explicitly when the
    /// application requires the pump stopped before releasing the transport.
    pub fn disconnect(&mut self) {
        self.transport = None;
        self.state = PumpState::default();
    }

    /// Consume the connection and return its transport without sending commands.
    ///
    /// Returns `None` if already disconnected. The caller takes responsibility
    /// for closing or reusing the transport.
    pub fn into_transport(self) -> Option<T> {
        self.transport
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> AsyncSf10<T> {
    /// Initialize an existing async transport with default options.
    ///
    /// Enables remote control, reads the firmware, and verifies remote-control
    /// status. Failed or cancelled initialization releases the transport.
    pub async fn from_transport(transport: T) -> Result<Self> {
        Self::from_transport_with_options(transport, ConnectionOptions::default()).await
    }

    /// Initialize an existing async transport and optionally set default values.
    ///
    /// `options.timeout` sets a whole-exchange deadline for every command,
    /// including each initialization command. The caller must configure and open
    /// the transport before calling this method. Requires a Tokio runtime with
    /// its time driver enabled.
    pub async fn from_transport_with_options(
        transport: T,
        options: ConnectionOptions,
    ) -> Result<Self> {
        let mut pump = Self {
            transport: Some(transport),
            state: PumpState::default(),
            firmware_version: String::new(),
            options,
        };
        pump.acknowledge("REMOTEEN vtec 1").await?;
        let firmware = pump.exchange("GV").await?;
        if firmware.is_empty() {
            return Err(VapourtecError::UnexpectedResponse {
                command: "GV".into(),
                response: firmware,
            });
        }
        pump.firmware_version = firmware;
        let response = pump.exchange("REMOTEEN").await?;
        if response != REMOTE_ENABLED {
            return Err(VapourtecError::UnexpectedResponse {
                command: "REMOTEEN".into(),
                response,
            });
        }
        if options.safe_connect {
            pump.initialize_defaults().await?;
        }
        Ok(pump)
    }

    /// Start the pump and record `running = Some(true)` after `OK`.
    pub async fn start(&mut self) -> Result<()> {
        self.acknowledge("START").await?;
        self.state.running = Some(true);
        Ok(())
    }

    /// Stop the pump and record `running = Some(false)` after `OK`.
    pub async fn stop(&mut self) -> Result<()> {
        self.acknowledge("STOP").await?;
        self.state.running = Some(false);
        Ok(())
    }

    /// Select inlet valve A or B.
    pub async fn set_valve(&mut self, valve: Valve) -> Result<()> {
        self.acknowledge(&format!("VALVE {valve}")).await?;
        self.state.valve = Some(valve);
        Ok(())
    }

    /// Select the pump's operating mode.
    pub async fn set_mode(&mut self, mode: Mode) -> Result<()> {
        self.acknowledge(&format!("MODE {mode}")).await?;
        self.state.mode = Some(mode);
        Ok(())
    }

    /// Set finite liquid flow in mL/min for FLOW and DOSE modes.
    ///
    /// Rounds to three decimal places with ties to even. The cached value is
    /// updated to the rounded request only after the command is acknowledged.
    pub async fn set_flow_rate(&mut self, flow_rate: f64) -> Result<()> {
        let rounded = rounded_flow(flow_rate)?;
        self.acknowledge(&format!("SETFLOW {}", number(rounded)))
            .await?;
        self.state.flow_rate = Some(rounded);
        Ok(())
    }

    /// Set finite gas flow in ssc/min for GAS mode.
    pub async fn set_gas_flow_rate(&mut self, flow_rate: f64) -> Result<()> {
        validate_finite("gas_flow_rate", flow_rate)?;
        self.acknowledge(&format!("SETGASFLOW {}", number(flow_rate)))
            .await?;
        self.state.gas_flow_rate = Some(flow_rate);
        Ok(())
    }

    /// Set finite regulator pressure in bar for REG mode.
    pub async fn set_regulator_pressure(&mut self, pressure: f64) -> Result<()> {
        validate_finite("regulator_pressure", pressure)?;
        self.acknowledge(&format!("SETREG {}", number(pressure)))
            .await?;
        self.state.regulator_pressure = Some(pressure);
        Ok(())
    }

    /// Set finite dose volume in mL for DOSE mode.
    pub async fn set_dose_volume(&mut self, volume: f64) -> Result<()> {
        validate_finite("dose_volume", volume)?;
        self.acknowledge(&format!("SETDOSE {}", number(volume)))
            .await?;
        self.state.dose_volume = Some(volume);
        Ok(())
    }

    /// Set initial/final flow in mL/min and duration in minutes for RAMP mode.
    ///
    /// All values must be finite. Cached fields update together only after the
    /// complete command is acknowledged.
    pub async fn set_ramp(&mut self, initial: f64, final_rate: f64, time: f64) -> Result<()> {
        validate_finite("ramp_initial_rate", initial)?;
        validate_finite("ramp_final_rate", final_rate)?;
        validate_finite("ramp_time", time)?;
        self.acknowledge(&format!(
            "SETRAMP {} {} {}",
            number(initial),
            number(final_rate),
            number(time)
        ))
        .await?;
        self.state.ramp_initial_rate = Some(initial);
        self.state.ramp_final_rate = Some(final_rate);
        self.state.ramp_time = Some(time);
        Ok(())
    }

    async fn initialize_defaults(&mut self) -> Result<()> {
        self.stop().await?;
        self.set_valve(Valve::A).await?;
        self.set_mode(Mode::Flow).await?;
        self.set_flow_rate(1.0).await?;
        self.set_gas_flow_rate(1.0).await?;
        self.set_regulator_pressure(1.0).await?;
        self.set_dose_volume(1.0).await?;
        self.set_ramp(0.5, 1.0, 1.0).await
    }

    async fn acknowledge(&mut self, command: &str) -> Result<()> {
        let response = self.exchange(command).await?;
        if response == "OK" {
            Ok(())
        } else {
            Err(VapourtecError::UnexpectedResponse {
                command: command.to_owned(),
                response,
            })
        }
    }

    async fn exchange(&mut self, command: &str) -> Result<String> {
        // Keep the transport inside this future while an exchange is in
        // flight. Cancellation drops it automatically, leaving the connection
        // disconnected and its state unknown. A complete, valid reply restores
        // the prior state before acknowledge() decides whether it was "OK".
        let mut transport = self.transport.take().ok_or(VapourtecError::NotConnected)?;
        let previous_state = std::mem::take(&mut self.state);
        let response =
            tokio::time::timeout(self.options.timeout, exchange(&mut transport, command))
                .await
                .map_err(|_| {
                    VapourtecError::Io(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "SF10 command exceeded its timeout",
                    ))
                })?;
        if response.is_ok() {
            self.transport = Some(transport);
            self.state = previous_state;
        }
        response
    }
}

async fn exchange<T: AsyncRead + AsyncWrite + Unpin>(
    transport: &mut T,
    command: &str,
) -> Result<String> {
    transport
        .write_all(format!("{command}\r").as_bytes())
        .await?;
    transport.flush().await?;
    let mut response = Vec::new();
    loop {
        let mut byte = [0];
        transport.read_exact(&mut byte).await?;
        if byte[0] == b'\r' {
            break;
        }
        if response.len() == MAX_RESPONSE_BYTES {
            return Err(VapourtecError::ResponseTooLong {
                limit: MAX_RESPONSE_BYTES,
            });
        }
        response.push(byte[0]);
    }
    decode_response(response)
}
