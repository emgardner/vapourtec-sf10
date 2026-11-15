use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::rc::Rc;

use vapourtec::{ConnectionOptions, Mode, Sf10, Valve, VapourtecError};

enum ReadStep {
    Bytes(VecDeque<u8>),
    Error(io::ErrorKind),
    Eof,
}

struct Exchange {
    command: Vec<u8>,
    response: VecDeque<ReadStep>,
}

impl Exchange {
    fn reply(command: &str, response: impl AsRef<[u8]>) -> Self {
        Self {
            command: format!("{command}\r").into_bytes(),
            response: VecDeque::from([ReadStep::Bytes(response.as_ref().to_vec().into())]),
        }
    }

    fn ok(command: &str) -> Self {
        Self::reply(command, b"OK\r")
    }

    fn steps(command: &str, response: Vec<ReadStep>) -> Self {
        Self {
            command: format!("{command}\r").into_bytes(),
            response: response.into(),
        }
    }
}

struct Script {
    exchanges: VecDeque<Exchange>,
    pending_command: Vec<u8>,
    reads: VecDeque<ReadStep>,
    commands: Vec<String>,
    max_write: usize,
    max_read: usize,
    write_error: Option<io::ErrorKind>,
    flush_error: Option<io::ErrorKind>,
    drops: usize,
}

struct MockTransport(Rc<RefCell<Script>>);

impl Read for MockTransport {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let mut script = self.0.borrow_mut();
        let max_read = script.max_read;
        loop {
            match script.reads.front_mut() {
                Some(ReadStep::Bytes(bytes)) if bytes.is_empty() => {
                    script.reads.pop_front();
                }
                Some(ReadStep::Bytes(bytes)) => {
                    let length = buffer.len().min(bytes.len()).min(max_read);
                    for byte in &mut buffer[..length] {
                        *byte = bytes.pop_front().unwrap();
                    }
                    return Ok(length);
                }
                Some(ReadStep::Error(kind)) => {
                    let kind = *kind;
                    script.reads.pop_front();
                    return Err(io::Error::from(kind));
                }
                Some(ReadStep::Eof) => {
                    script.reads.pop_front();
                    return Ok(0);
                }
                None => return Err(io::Error::from(io::ErrorKind::WouldBlock)),
            }
        }
    }
}

impl Write for MockTransport {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut script = self.0.borrow_mut();
        if let Some(kind) = script.write_error.take() {
            return Err(io::Error::from(kind));
        }
        let count = bytes.len().min(script.max_write);
        for &byte in &bytes[..count] {
            script.pending_command.push(byte);
            let expected = &script
                .exchanges
                .front()
                .expect("library sent an unexpected command")
                .command;
            assert!(
                expected.starts_with(&script.pending_command),
                "unexpected command prefix: {:?}; expected {:?}",
                String::from_utf8_lossy(&script.pending_command),
                String::from_utf8_lossy(expected)
            );
            if byte == b'\r' {
                let exchange = script.exchanges.pop_front().unwrap();
                assert_eq!(script.pending_command, exchange.command);
                let command = String::from_utf8(script.pending_command.clone()).unwrap();
                script.commands.push(command);
                script.pending_command.clear();
                script.reads.extend(exchange.response);
            }
        }
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.0.borrow_mut().flush_error.take() {
            Some(kind) => Err(io::Error::from(kind)),
            None => Ok(()),
        }
    }
}

impl Drop for MockTransport {
    fn drop(&mut self) {
        self.0.borrow_mut().drops += 1;
    }
}

fn handshake() -> Vec<Exchange> {
    vec![
        Exchange::ok("REMOTEEN vtec 1"),
        Exchange::reply("GV", b"SF10 v1.8\r"),
        Exchange::reply("REMOTEEN", b"Remote Control is enabled\r"),
    ]
}

fn mock(exchanges: Vec<Exchange>) -> (MockTransport, Rc<RefCell<Script>>) {
    let script = Rc::new(RefCell::new(Script {
        exchanges: exchanges.into(),
        pending_command: Vec::new(),
        reads: VecDeque::new(),
        commands: Vec::new(),
        max_write: usize::MAX,
        max_read: usize::MAX,
        write_error: None,
        flush_error: None,
        drops: 0,
    }));
    (MockTransport(Rc::clone(&script)), script)
}

fn connected(extra: Vec<Exchange>) -> (Sf10<MockTransport>, Rc<RefCell<Script>>) {
    let mut exchanges = handshake();
    exchanges.extend(extra);
    let (transport, script) = mock(exchanges);
    (Sf10::from_transport(transport).unwrap(), script)
}

#[test]
fn connects_with_exact_handshake_and_unknown_initial_state() {
    let (pump, script) = connected(vec![]);
    assert_eq!(pump.firmware_version(), "SF10 v1.8");
    assert!(pump.is_connected());
    assert_eq!(*pump.state(), Default::default());
    assert_eq!(
        script.borrow().commands,
        ["REMOTEEN vtec 1\r", "GV\r", "REMOTEEN\r"]
    );
    assert!(script.borrow().exchanges.is_empty());
}

#[test]
fn all_controls_use_protocol_tokens_and_track_acknowledged_values() {
    let (mut pump, script) = connected(vec![
        Exchange::ok("START"),
        Exchange::ok("STOP"),
        Exchange::ok("VALVE A"),
        Exchange::ok("VALVE B"),
        Exchange::ok("MODE FLOW"),
        Exchange::ok("MODE REG"),
        Exchange::ok("MODE DOSE"),
        Exchange::ok("MODE GAS"),
        Exchange::ok("MODE RAMP"),
        Exchange::ok("SETFLOW 1.234"),
        Exchange::ok("SETGASFLOW 50.12345"),
        Exchange::ok("SETREG 4.12345"),
        Exchange::ok("SETDOSE 20.12345"),
        Exchange::ok("SETRAMP 0.5 2.0 10.0"),
    ]);

    pump.start().unwrap();
    assert_eq!(pump.state().running, Some(true));
    pump.stop().unwrap();
    assert_eq!(pump.state().running, Some(false));
    for valve in [Valve::A, Valve::B] {
        pump.set_valve(valve).unwrap();
        assert_eq!(pump.state().valve, Some(valve));
    }
    for mode in [
        Mode::Flow,
        Mode::Regulator,
        Mode::Dose,
        Mode::Gas,
        Mode::Ramp,
    ] {
        pump.set_mode(mode).unwrap();
        assert_eq!(pump.state().mode, Some(mode));
    }
    pump.set_flow_rate(1.2344).unwrap();
    pump.set_gas_flow_rate(50.12345).unwrap();
    pump.set_regulator_pressure(4.12345).unwrap();
    pump.set_dose_volume(20.12345).unwrap();
    pump.set_ramp(0.5, 2.0, 10.0).unwrap();

    let state = pump.state();
    assert_eq!(state.flow_rate, Some(1.234));
    assert_eq!(state.gas_flow_rate, Some(50.12345));
    assert_eq!(state.regulator_pressure, Some(4.12345));
    assert_eq!(state.dose_volume, Some(20.12345));
    assert_eq!(state.ramp_initial_rate, Some(0.5));
    assert_eq!(state.ramp_final_rate, Some(2.0));
    assert_eq!(state.ramp_time, Some(10.0));
    assert!(script.borrow().exchanges.is_empty());
}

#[test]
fn flow_rounding_matches_python_decimal_ties_to_even() {
    let (mut pump, _) = connected(vec![
        Exchange::ok("SETFLOW 1.062"),
        Exchange::ok("SETFLOW 1.188"),
        Exchange::ok("SETFLOW 2.0"),
    ]);
    pump.set_flow_rate(1.0625).unwrap();
    assert_eq!(pump.state().flow_rate, Some(1.062));
    pump.set_flow_rate(1.1875).unwrap();
    assert_eq!(pump.state().flow_rate, Some(1.188));
    pump.set_flow_rate(1.9999).unwrap();
    assert_eq!(pump.state().flow_rate, Some(2.0));
}

#[test]
fn safe_connect_stops_and_initializes_every_cached_setting() {
    let mut exchanges = handshake();
    exchanges.extend([
        Exchange::ok("STOP"),
        Exchange::ok("VALVE A"),
        Exchange::ok("MODE FLOW"),
        Exchange::ok("SETFLOW 1.0"),
        Exchange::ok("SETGASFLOW 1.0"),
        Exchange::ok("SETREG 1.0"),
        Exchange::ok("SETDOSE 1.0"),
        Exchange::ok("SETRAMP 0.5 1.0 1.0"),
    ]);
    let (transport, script) = mock(exchanges);
    let pump = Sf10::from_transport_with_options(
        transport,
        ConnectionOptions {
            safe_connect: true,
            ..Default::default()
        },
    )
    .unwrap();
    let state = pump.state();
    assert_eq!(state.running, Some(false));
    assert_eq!(state.valve, Some(Valve::A));
    assert_eq!(state.mode, Some(Mode::Flow));
    assert_eq!(state.flow_rate, Some(1.0));
    assert_eq!(state.gas_flow_rate, Some(1.0));
    assert_eq!(state.regulator_pressure, Some(1.0));
    assert_eq!(state.dose_volume, Some(1.0));
    assert_eq!(state.ramp_initial_rate, Some(0.5));
    assert_eq!(state.ramp_final_rate, Some(1.0));
    assert_eq!(state.ramp_time, Some(1.0));
    assert!(script.borrow().exchanges.is_empty());
}

#[test]
fn safe_connect_fails_immediately_when_stop_is_rejected() {
    let mut exchanges = handshake();
    exchanges.push(Exchange::reply("STOP", b"ERROR\r"));
    let (transport, script) = mock(exchanges);
    let result = Sf10::from_transport_with_options(
        transport,
        ConnectionOptions {
            safe_connect: true,
            ..Default::default()
        },
    );
    assert!(matches!(
        result,
        Err(VapourtecError::UnexpectedResponse { command, response })
            if command == "STOP" && response == "ERROR"
    ));
    assert_eq!(script.borrow().commands.len(), 4);
    assert_eq!(script.borrow().drops, 1);
}

#[test]
fn connection_rejects_failed_remote_enable_before_querying_firmware() {
    let (transport, script) = mock(vec![Exchange::reply("REMOTEEN vtec 1", b"ERROR\r")]);
    let result = Sf10::from_transport(transport);
    assert!(matches!(
        result,
        Err(VapourtecError::UnexpectedResponse { command, response })
            if command == "REMOTEEN vtec 1" && response == "ERROR"
    ));
    assert_eq!(script.borrow().commands.len(), 1);
    assert_eq!(script.borrow().drops, 1);
}

#[test]
fn connection_requires_remote_control_to_be_enabled() {
    let (transport, script) = mock(vec![
        Exchange::ok("REMOTEEN vtec 1"),
        Exchange::reply("GV", b"SF10 v1.8\r"),
        Exchange::reply("REMOTEEN", b"Remote Control is disabled\r"),
    ]);
    assert!(matches!(
        Sf10::from_transport(transport),
        Err(VapourtecError::UnexpectedResponse { command, .. }) if command == "REMOTEEN"
    ));
    assert_eq!(script.borrow().drops, 1);
}

#[test]
fn connection_rejects_empty_firmware_response() {
    let (transport, script) = mock(vec![
        Exchange::ok("REMOTEEN vtec 1"),
        Exchange::reply("GV", b"\r"),
    ]);
    assert!(matches!(
        Sf10::from_transport(transport),
        Err(VapourtecError::UnexpectedResponse { command, .. }) if command == "GV"
    ));
    assert_eq!(script.borrow().drops, 1);
}

#[test]
fn rejection_preserves_previous_state_and_connection() {
    let (mut pump, script) = connected(vec![
        Exchange::ok("SETFLOW 2.0"),
        Exchange::reply("SETFLOW 3.0", b"ERROR\r"),
        Exchange::reply("START", b"ok\r"),
        Exchange::ok("STOP"),
    ]);
    pump.set_flow_rate(2.0).unwrap();
    assert!(matches!(
        pump.set_flow_rate(3.0),
        Err(VapourtecError::UnexpectedResponse { command, response })
            if command == "SETFLOW 3.0" && response == "ERROR"
    ));
    assert_eq!(pump.state().flow_rate, Some(2.0));
    assert!(matches!(
        pump.start(),
        Err(VapourtecError::UnexpectedResponse { response, .. }) if response == "ok"
    ));
    assert_eq!(pump.state().running, None);
    assert!(pump.is_connected());
    pump.stop().unwrap();
    assert_eq!(script.borrow().drops, 0);
}

#[test]
fn non_finite_numbers_are_rejected_without_sending_or_changing_state() {
    let (mut pump, script) = connected(vec![]);
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(matches!(
            pump.set_flow_rate(value),
            Err(VapourtecError::InvalidParameter { .. })
        ));
        assert!(matches!(
            pump.set_gas_flow_rate(value),
            Err(VapourtecError::InvalidParameter { .. })
        ));
        assert!(matches!(
            pump.set_regulator_pressure(value),
            Err(VapourtecError::InvalidParameter { .. })
        ));
        assert!(matches!(
            pump.set_dose_volume(value),
            Err(VapourtecError::InvalidParameter { .. })
        ));
        for ramp in [(value, 1.0, 1.0), (1.0, value, 1.0), (1.0, 1.0, value)] {
            assert!(matches!(
                pump.set_ramp(ramp.0, ramp.1, ramp.2),
                Err(VapourtecError::InvalidParameter { .. })
            ));
        }
    }
    assert!(pump.is_connected());
    assert_eq!(*pump.state(), Default::default());
    assert_eq!(script.borrow().commands.len(), 3);
}

#[test]
fn finite_negative_values_do_not_invent_undocumented_hardware_limits() {
    let (mut pump, _) = connected(vec![Exchange::ok("SETFLOW -1.0")]);
    pump.set_flow_rate(-1.0).unwrap();
    assert_eq!(pump.state().flow_rate, Some(-1.0));
}

#[test]
fn framing_handles_short_writes_fragmented_reads_and_exterior_line_feeds() {
    let (transport, script) = mock(vec![
        Exchange::reply("REMOTEEN vtec 1", b"\nOK\r\n"),
        Exchange::reply("GV", b"SF10 v1.8\n\r"),
        Exchange::reply("REMOTEEN", b"\nRemote Control is enabled\r\n"),
        Exchange::reply("START", b"\nOK\n\r"),
    ]);
    script.borrow_mut().max_write = 2;
    script.borrow_mut().max_read = 1;
    let mut pump = Sf10::from_transport(transport).unwrap();
    assert_eq!(pump.firmware_version(), "SF10 v1.8");
    pump.start().unwrap();
    assert_eq!(pump.state().running, Some(true));
    assert!(script.borrow().exchanges.is_empty());
}

#[test]
fn timeout_after_partial_response_disconnects_and_invalidates_cached_state() {
    let (mut pump, script) = connected(vec![
        Exchange::ok("START"),
        Exchange::steps(
            "SETFLOW 2.0",
            vec![
                ReadStep::Bytes(b"O".to_vec().into()),
                ReadStep::Error(io::ErrorKind::TimedOut),
                ReadStep::Bytes(b"K\r".to_vec().into()),
            ],
        ),
    ]);
    pump.start().unwrap();
    assert!(matches!(
        pump.set_flow_rate(2.0),
        Err(VapourtecError::Io(error)) if error.kind() == io::ErrorKind::TimedOut
    ));
    assert!(!pump.is_connected());
    assert_eq!(*pump.state(), Default::default());
    assert_eq!(script.borrow().drops, 1);
    assert!(matches!(pump.stop(), Err(VapourtecError::NotConnected)));
    assert_eq!(script.borrow().commands.len(), 5);
}

#[test]
fn end_of_stream_is_an_io_error_and_disconnects() {
    let (mut pump, script) = connected(vec![Exchange::steps("START", vec![ReadStep::Eof])]);
    assert!(matches!(
        pump.start(),
        Err(VapourtecError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof
    ));
    assert!(!pump.is_connected());
    assert_eq!(script.borrow().drops, 1);
}

#[test]
fn write_failure_disconnects_and_preserves_the_io_cause() {
    let (mut pump, script) = connected(vec![]);
    script.borrow_mut().write_error = Some(io::ErrorKind::BrokenPipe);
    assert!(matches!(
        pump.start(),
        Err(VapourtecError::Io(error)) if error.kind() == io::ErrorKind::BrokenPipe
    ));
    assert!(!pump.is_connected());
    assert_eq!(script.borrow().drops, 1);
}

#[test]
fn invalid_utf8_disconnects_instead_of_losing_response_alignment() {
    let (mut pump, script) = connected(vec![Exchange::reply("START", b"\xff\r")]);
    assert!(matches!(
        pump.start(),
        Err(VapourtecError::InvalidResponse(_))
    ));
    assert!(!pump.is_connected());
    assert_eq!(script.borrow().drops, 1);
}

#[test]
fn oversized_response_is_bounded_and_disconnects() {
    let (mut pump, script) = connected(vec![Exchange::reply("START", vec![b'x'; 1025])]);
    assert!(matches!(
        pump.start(),
        Err(VapourtecError::ResponseTooLong { limit: 1024 })
    ));
    assert!(!pump.is_connected());
    assert_eq!(script.borrow().drops, 1);
}

#[test]
fn disconnect_is_idempotent_clears_state_and_never_sends_stop() {
    let (mut pump, script) = connected(vec![Exchange::ok("START")]);
    pump.start().unwrap();
    pump.disconnect();
    pump.disconnect();
    assert!(!pump.is_connected());
    assert_eq!(*pump.state(), Default::default());
    assert_eq!(script.borrow().drops, 1);
    assert_eq!(script.borrow().commands.len(), 4);
    assert!(matches!(pump.stop(), Err(VapourtecError::NotConnected)));
    assert!(pump.into_transport().is_none());
}

#[test]
fn dropping_connected_pump_only_releases_transport() {
    let (mut pump, script) = connected(vec![Exchange::ok("START")]);
    pump.start().unwrap();
    drop(pump);
    assert_eq!(script.borrow().drops, 1);
    assert_eq!(script.borrow().commands.len(), 4);
}

#[test]
fn into_transport_returns_ownership_without_extra_protocol_commands() {
    let (pump, script) = connected(vec![]);
    let transport = pump.into_transport().unwrap();
    assert_eq!(script.borrow().drops, 0);
    assert_eq!(script.borrow().commands.len(), 3);
    drop(transport);
    assert_eq!(script.borrow().drops, 1);
}
