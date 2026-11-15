use crate::{Result, VapourtecError};

pub(crate) const MAX_RESPONSE_BYTES: usize = 1_024;
pub(crate) const REMOTE_ENABLED: &str = "Remote Control is enabled";

pub(crate) fn validate_finite(parameter: &'static str, value: f64) -> Result<()> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(VapourtecError::InvalidParameter { parameter, value })
    }
}

pub(crate) fn rounded_flow(value: f64) -> Result<f64> {
    validate_finite("flow_rate", value)?;
    // Decimal formatting matches Python's ties-to-even rounding of binary
    // floats without overflowing large finite inputs by multiplying by 1000.
    Ok(format!("{value:.3}")
        .parse::<f64>()
        .expect("a finite float formatted to three decimals is a finite float"))
}

pub(crate) fn number(value: f64) -> String {
    // Preserve the Python float command spelling for integral f64 values.
    let mut formatted = value.to_string();
    if !formatted.contains('.') {
        formatted.push_str(".0");
    }
    formatted
}

pub(crate) fn decode_response(response: Vec<u8>) -> Result<String> {
    let response = String::from_utf8(response)?;
    // Some cables prepend LF; CRLF also leaves LF for the next exchange.
    Ok(response.trim_matches('\n').to_owned())
}
