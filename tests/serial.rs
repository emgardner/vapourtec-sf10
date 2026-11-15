#![cfg(all(unix, feature = "serialport"))]

use std::io::{Read, Write};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[cfg(target_os = "linux")]
use serialport::{DataBits, FlowControl, Parity, StopBits};
use serialport::{SerialPort, TTYPort};
#[cfg(target_os = "linux")]
use vapourtec::{open_serial_port, ConnectionOptions, Mode, Valve};
use vapourtec::{Sf10, VapourtecError};

type Reply = (&'static str, Option<&'static str>);

fn handshake() -> Vec<Reply> {
    vec![
        ("REMOTEEN vtec 1", Some("OK\r")),
        ("GV", Some("SF10 v1.8\r")),
        ("REMOTEEN", Some("Remote Control is enabled\r")),
    ]
}

#[cfg(target_os = "linux")]
fn safe_handshake() -> Vec<Reply> {
    let mut replies = handshake();
    replies.extend([
        ("STOP", Some("OK\r")),
        ("VALVE A", Some("OK\r")),
        ("MODE FLOW", Some("OK\r")),
        ("SETFLOW 1.0", Some("OK\r")),
        ("SETGASFLOW 1.0", Some("OK\r")),
        ("SETREG 1.0", Some("OK\r")),
        ("SETDOSE 1.0", Some("OK\r")),
        ("SETRAMP 0.5 1.0 1.0", Some("OK\r")),
    ]);
    replies
}

struct SimulatedPump {
    #[cfg(target_os = "linux")]
    port_name: String,
    // pair() does not acquire an actual advisory/exclusive lock. Keeping this
    // handle open avoids a no-slave interval during native open and reconnect.
    slave_anchor: Option<TTYPort>,
    // The thread returns the master so it remains open until finish(), including
    // when a test intentionally withholds a response to trigger an I/O timeout.
    worker: JoinHandle<TTYPort>,
}

impl SimulatedPump {
    fn new(replies: Vec<Reply>) -> Self {
        let (mut master, slave) = TTYPort::pair().expect("create pseudo-terminal pair");
        master
            .set_timeout(Duration::from_secs(3))
            .expect("configure a bounded simulator read timeout");
        #[cfg(target_os = "linux")]
        let port_name = slave.name().expect("pseudo-terminal slave has a path");
        let worker = thread::spawn(move || {
            for (expected, reply) in replies {
                let mut command = Vec::new();
                loop {
                    let mut byte = [0];
                    master
                        .read_exact(&mut byte)
                        .expect("receive the next command before the simulator timeout");
                    if byte[0] == b'\r' {
                        break;
                    }
                    command.push(byte[0]);
                    assert!(command.len() < 256, "command did not terminate with CR");
                }
                assert_eq!(String::from_utf8(command).unwrap(), expected);
                if let Some(reply) = reply {
                    master
                        .write_all(reply.as_bytes())
                        .expect("send the response to the received command");
                }
            }
            master
        });
        Self {
            #[cfg(target_os = "linux")]
            port_name,
            slave_anchor: Some(slave),
            worker,
        }
    }

    fn finish(self) {
        drop(self.worker.join().expect("simulated pump completed"));
        drop(self.slave_anchor);
    }
}

#[test]
// serialport uses IOSSIOSPEED when opening a macOS/iOS port at 9600 baud;
// pseudo-terminals do not implement that ioctl. Real TTY I/O is tested below.
#[cfg(target_os = "linux")]
fn native_serial_connect_and_controls_use_configured_port() {
    let mut replies = handshake();
    replies.extend([
        ("VALVE B", Some("OK\r")),
        ("SETFLOW 2.5", Some("OK\r")),
        ("START", Some("OK\r")),
        ("STOP", Some("OK\r")),
    ]);
    let simulator = SimulatedPump::new(replies);
    let port = open_serial_port(&simulator.port_name, Duration::from_secs(1)).unwrap();
    let mut pump = Sf10::from_transport(port).unwrap();
    assert_eq!(pump.firmware_version(), "SF10 v1.8");
    pump.set_valve(Valve::B).unwrap();
    pump.set_flow_rate(2.5).unwrap();
    pump.start().unwrap();
    pump.stop().unwrap();
    assert_eq!(pump.state().running, Some(false));
    assert_eq!(pump.state().valve, Some(Valve::B));
    assert_eq!(pump.state().flow_rate, Some(2.5));

    let port = pump.into_transport().unwrap();
    assert_eq!(port.baud_rate().unwrap(), 9600);
    assert_eq!(port.data_bits().unwrap(), DataBits::Eight);
    assert_eq!(port.parity().unwrap(), Parity::None);
    assert_eq!(port.stop_bits().unwrap(), StopBits::One);
    assert_eq!(port.flow_control().unwrap(), FlowControl::None);
    assert_eq!(port.timeout(), Duration::from_secs(1));
    drop(port);
    simulator.finish();
}

#[test]
#[cfg(target_os = "linux")]
fn reopening_transport_repeats_handshake_and_safe_connect_options() {
    let mut replies = safe_handshake();
    replies.push(("START", Some("OK\r")));
    replies.extend(safe_handshake());
    let simulator = SimulatedPump::new(replies);
    let options = ConnectionOptions {
        safe_connect: true,
        timeout: Duration::from_millis(1500),
    };
    let port = open_serial_port(&simulator.port_name, options.timeout).unwrap();
    let mut pump = Sf10::from_transport_with_options(port, options).unwrap();
    pump.start().unwrap();
    assert_eq!(pump.state().running, Some(true));
    pump.disconnect();
    let port = open_serial_port(&simulator.port_name, options.timeout).unwrap();
    let pump = Sf10::from_transport_with_options(port, options).unwrap();

    assert!(pump.is_connected());
    assert_eq!(pump.state().running, Some(false));
    assert_eq!(pump.state().valve, Some(Valve::A));
    assert_eq!(pump.state().mode, Some(Mode::Flow));
    assert_eq!(pump.state().regulator_pressure, Some(1.0));
    assert_eq!(pump.state().ramp_initial_rate, Some(0.5));
    let port = pump.into_transport().unwrap();
    assert_eq!(port.timeout(), options.timeout);
    drop(port);
    simulator.finish();
}

#[test]
fn serial_read_timeout_releases_port_and_invalidates_state() {
    let mut replies = handshake();
    replies.extend([("SETFLOW 2.0", Some("OK\r")), ("START", None)]);
    let mut simulator = SimulatedPump::new(replies);
    let mut transport = simulator.slave_anchor.take().unwrap();
    transport.set_timeout(Duration::from_millis(100)).unwrap();
    let mut pump = Sf10::from_transport(transport).unwrap();
    pump.set_flow_rate(2.0).unwrap();
    assert_eq!(pump.state().flow_rate, Some(2.0));
    assert!(matches!(
        pump.start(),
        Err(VapourtecError::Io(error)) if error.kind() == std::io::ErrorKind::TimedOut
    ));
    assert!(!pump.is_connected());
    assert_eq!(*pump.state(), Default::default());
    assert!(matches!(pump.stop(), Err(VapourtecError::NotConnected)));
    assert!(pump.into_transport().is_none());
    simulator.finish();
}
