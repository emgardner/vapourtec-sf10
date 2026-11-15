use std::time::Duration;

/// Initialization and timeout settings for an SF10 client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionOptions {
    /// Stop and initialize every setting after the connection handshake.
    ///
    /// Defaults to `false`, leaving settings unknown until acknowledged. When
    /// enabled, selects valve A and FLOW mode, sets liquid/gas flow, regulator
    /// pressure and dose volume to 1.0, and sets the ramp to 0.5 → 1.0 mL/min
    /// over 1.0 minute. A rejected initialization command fails the connection.
    pub safe_connect: bool,
    /// Deadline for each complete async command, including writing and reading.
    ///
    /// Defaults to one second. Blocking transports must configure their own I/O
    /// timeouts; pass this value to `open_serial_port` when using that helper.
    /// Arbitrary `Read` + `Write` transports cannot be configured by the client.
    pub timeout: Duration,
}

impl Default for ConnectionOptions {
    fn default() -> Self {
        Self {
            safe_connect: false,
            timeout: Duration::from_secs(1),
        }
    }
}
