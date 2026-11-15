use std::fmt;

/// Inlet valve position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Valve {
    /// Inlet A.
    A,
    /// Inlet B.
    B,
}

impl fmt::Display for Valve {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::A => "A",
            Self::B => "B",
        })
    }
}

/// SF10 operating mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Constant liquid flow (`FLOW`).
    Flow,
    /// Pressure regulation (`REG`).
    Regulator,
    /// Dose dispensing (`DOSE`).
    Dose,
    /// Gas flow (`GAS`).
    Gas,
    /// Timed liquid flow ramp (`RAMP`).
    Ramp,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Flow => "FLOW",
            Self::Regulator => "REG",
            Self::Dose => "DOSE",
            Self::Gas => "GAS",
            Self::Ramp => "RAMP",
        })
    }
}

/// Last acknowledged requests on this connection, not physical readback.
///
/// The SF10 cannot report these settings and may acknowledge numerical values
/// outside its supported range without applying them. `None` means unknown.
/// Values start unknown and are cleared on disconnect or an I/O/framing error.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PumpState {
    /// Whether the last acknowledged command was `START` or `STOP`.
    pub running: Option<bool>,
    /// Last requested inlet valve position.
    pub valve: Option<Valve>,
    /// Last requested operating mode.
    pub mode: Option<Mode>,
    /// Liquid flow in mL/min, shared by FLOW and DOSE modes.
    pub flow_rate: Option<f64>,
    /// Gas flow in ssc/min, as named in the original protocol documentation.
    pub gas_flow_rate: Option<f64>,
    /// Regulator pressure in bar.
    pub regulator_pressure: Option<f64>,
    /// Dose volume in mL.
    pub dose_volume: Option<f64>,
    /// Initial ramp flow in mL/min.
    pub ramp_initial_rate: Option<f64>,
    /// Final ramp flow in mL/min.
    pub ramp_final_rate: Option<f64>,
    /// Ramp duration in minutes.
    pub ramp_time: Option<f64>,
}
