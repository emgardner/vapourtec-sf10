use std::io::{Read, Write};

use crate::protocol::{
    decode_response, number, rounded_flow, validate_finite, MAX_RESPONSE_BYTES, REMOTE_ENABLED,
};
use crate::{ConnectionOptions, Mode, PumpState, Result, Valve, VapourtecError};

/// A synchronous SF10 pump connection.
///
/// Accepts any [`Read`] + [`Write`] transport through [`Self::from_transport`].
/// Commands take `&mut self` so
/// their request/reply exchanges cannot interleave through this instance.
///
/// Dropping the instance closes its transport without sending any commands.
pub struct Sf10<T> {
    transport: Option<T>,
    state: PumpState,
    firmware_version: String,
}

impl<T: Read + Write> Sf10<T> {
    /// Initialize an existing blocking transport with default options.
    ///
    /// The caller configures the transport, including framing settings and I/O
    /// timeouts. Takes ownership and performs the same remote-control handshake
    /// as the Python implementation. Failed initialization drops the transport.
    pub fn from_transport(transport: T) -> Result<Self> {
        Self::from_transport_with_options(transport, ConnectionOptions::default())
    }

    /// Initialize an existing transport and optionally establish default settings.
    ///
    /// Configure I/O timeouts on the transport before transferring ownership
    /// here. `options.timeout` is used only by the async client.
    pub fn from_transport_with_options(transport: T, options: ConnectionOptions) -> Result<Self> {
        let mut pump = Self {
            transport: Some(transport),
            state: PumpState::default(),
            firmware_version: String::new(),
        };
        pump.acknowledge("REMOTEEN vtec 1")?;
        let firmware = pump.exchange("GV")?;
        if firmware.is_empty() {
            return Err(VapourtecError::UnexpectedResponse {
                command: "GV".into(),
                response: firmware,
            });
        }
        pump.firmware_version = firmware;
        let response = pump.exchange("REMOTEEN")?;
        if response != REMOTE_ENABLED {
            return Err(VapourtecError::UnexpectedResponse {
                command: "REMOTEEN".into(),
                response,
            });
        }
        if options.safe_connect {
            pump.initialize_defaults()?;
        }
        Ok(pump)
    }

    /// Return the last acknowledged requests. Unknown settings are `None`.
    pub fn state(&self) -> &PumpState {
        &self.state
    }

    /// Return the unparsed firmware reply obtained during the last handshake.
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

    /// Close the transport and clear cached settings. Repeated calls are harmless.
    ///
    /// Does not stop the pump or send a remote-control-disable command. Call
    /// [`Self::stop`] explicitly if the application requires the pump stopped.
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

    /// Start the pump and record `running = Some(true)` after `OK`.
    pub fn start(&mut self) -> Result<()> {
        self.acknowledge("START")?;
        self.state.running = Some(true);
        Ok(())
    }

    /// Stop the pump and record `running = Some(false)` after `OK`.
    pub fn stop(&mut self) -> Result<()> {
        self.acknowledge("STOP")?;
        self.state.running = Some(false);
        Ok(())
    }

    /// Select inlet valve A or B.
    pub fn set_valve(&mut self, valve: Valve) -> Result<()> {
        self.acknowledge(&format!("VALVE {valve}"))?;
        self.state.valve = Some(valve);
        Ok(())
    }

    /// Select the pump's operating mode.
    pub fn set_mode(&mut self, mode: Mode) -> Result<()> {
        self.acknowledge(&format!("MODE {mode}"))?;
        self.state.mode = Some(mode);
        Ok(())
    }

    /// Set liquid flow in mL/min for FLOW and DOSE modes.
    ///
    /// Rounds to three decimal places with ties to even, matching the Python
    /// implementation. The returned state contains the rounded value. Rejects
    /// NaN and infinity; physical range limits are enforced by the pump.
    pub fn set_flow_rate(&mut self, flow_rate: f64) -> Result<()> {
        let rounded = rounded_flow(flow_rate)?;
        self.acknowledge(&format!("SETFLOW {}", number(rounded)))?;
        self.state.flow_rate = Some(rounded);
        Ok(())
    }

    /// Set gas flow in ssc/min for GAS mode. Requires a finite value.
    pub fn set_gas_flow_rate(&mut self, flow_rate: f64) -> Result<()> {
        validate_finite("gas_flow_rate", flow_rate)?;
        self.acknowledge(&format!("SETGASFLOW {}", number(flow_rate)))?;
        self.state.gas_flow_rate = Some(flow_rate);
        Ok(())
    }

    /// Set regulator pressure in bar for REG mode. Requires a finite value.
    pub fn set_regulator_pressure(&mut self, pressure: f64) -> Result<()> {
        validate_finite("regulator_pressure", pressure)?;
        self.acknowledge(&format!("SETREG {}", number(pressure)))?;
        self.state.regulator_pressure = Some(pressure);
        Ok(())
    }

    /// Set dose volume in mL for DOSE mode. Requires a finite value.
    pub fn set_dose_volume(&mut self, volume: f64) -> Result<()> {
        validate_finite("dose_volume", volume)?;
        self.acknowledge(&format!("SETDOSE {}", number(volume)))?;
        self.state.dose_volume = Some(volume);
        Ok(())
    }

    /// Set initial/final flow in mL/min and duration in minutes for RAMP mode.
    ///
    /// All three values must be finite. Cached ramp fields update together only
    /// after the complete command is acknowledged.
    pub fn set_ramp(&mut self, initial: f64, final_rate: f64, time: f64) -> Result<()> {
        validate_finite("ramp_initial_rate", initial)?;
        validate_finite("ramp_final_rate", final_rate)?;
        validate_finite("ramp_time", time)?;
        self.acknowledge(&format!(
            "SETRAMP {} {} {}",
            number(initial),
            number(final_rate),
            number(time)
        ))?;
        self.state.ramp_initial_rate = Some(initial);
        self.state.ramp_final_rate = Some(final_rate);
        self.state.ramp_time = Some(time);
        Ok(())
    }

    fn initialize_defaults(&mut self) -> Result<()> {
        self.stop()?;
        self.set_valve(Valve::A)?;
        self.set_mode(Mode::Flow)?;
        self.set_flow_rate(1.0)?;
        self.set_gas_flow_rate(1.0)?;
        // The Python implementation cached 1.0 without actually sending SETREG.
        self.set_regulator_pressure(1.0)?;
        self.set_dose_volume(1.0)?;
        self.set_ramp(0.5, 1.0, 1.0)
    }

    fn acknowledge(&mut self, command: &str) -> Result<()> {
        let response = self.exchange(command)?;
        if response == "OK" {
            Ok(())
        } else {
            Err(VapourtecError::UnexpectedResponse {
                command: command.to_owned(),
                response,
            })
        }
    }

    fn exchange(&mut self, command: &str) -> Result<String> {
        let transport = self
            .transport
            .as_mut()
            .ok_or(VapourtecError::NotConnected)?;
        let response = exchange(transport, command);
        if response.is_err() {
            // A timeout or partial frame can leave a late reply queued. Reusing
            // it for the next command would corrupt acknowledgement tracking.
            self.disconnect();
        }
        response
    }
}

fn exchange(transport: &mut (impl Read + Write), command: &str) -> Result<String> {
    transport.write_all(format!("{command}\r").as_bytes())?;
    transport.flush()?;
    let mut response = Vec::new();
    loop {
        let mut byte = [0];
        transport.read_exact(&mut byte)?;
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
