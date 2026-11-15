#![cfg(feature = "async")]

use std::{io, time::Duration};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    sync::mpsc,
    task::JoinHandle,
};
use vapourtec::{AsyncSf10, ConnectionOptions, Mode, Valve, VapourtecError};

struct Exchange {
    command: &'static str,
    response: Vec<u8>,
    eof: bool,
}

impl Exchange {
    fn reply(command: &'static str, response: impl AsRef<[u8]>) -> Self {
        Self {
            command,
            response: response.as_ref().to_vec(),
            eof: false,
        }
    }

    fn ok(command: &'static str) -> Self {
        Self::reply(command, b"OK\r")
    }

    fn eof(command: &'static str) -> Self {
        Self {
            eof: true,
            ..Self::reply(command, [])
        }
    }
}

fn handshake() -> Vec<Exchange> {
    vec![
        Exchange::ok("REMOTEEN vtec 1"),
        Exchange::reply("GV", b"SF10 v1.8\r"),
        Exchange::reply("REMOTEEN", b"Remote Control is enabled\r"),
    ]
}

struct Simulator {
    worker: JoinHandle<DuplexStream>,
    commands: mpsc::UnboundedReceiver<String>,
}

impl Simulator {
    fn new(exchanges: Vec<Exchange>) -> (DuplexStream, Self) {
        // The small buffer forces partial writes in both directions.
        let (client, mut server) = tokio::io::duplex(16);
        let (commands_tx, commands) = mpsc::unbounded_channel();
        let worker = tokio::spawn(async move {
            for exchange in exchanges {
                let mut command = Vec::new();
                loop {
                    let byte = server.read_u8().await.expect("read command byte");
                    if byte == b'\r' {
                        break;
                    }
                    command.push(byte);
                    assert!(command.len() < 256, "command did not terminate");
                }
                let command = String::from_utf8(command).unwrap();
                assert_eq!(command, exchange.command);
                // Each byte is sent separately to exercise fragmented replies.
                for byte in exchange.response {
                    server.write_u8(byte).await.expect("write response byte");
                }
                if exchange.eof {
                    server.shutdown().await.unwrap();
                }
                // Notify only after the reply has been written so cancellation
                // tests cannot close the peer before a scripted partial reply.
                commands_tx.send(command).unwrap();
            }
            // Returning the stream keeps the peer alive in the join result
            // until the test finishes or explicitly checks for disconnection.
            server
        });
        (client, Self { worker, commands })
    }

    async fn finish(self) {
        self.worker.await.expect("simulated pump completed");
    }
}

async fn connected(extra: Vec<Exchange>) -> (AsyncSf10<DuplexStream>, Simulator) {
    let mut exchanges = handshake();
    exchanges.extend(extra);
    let (transport, simulator) = Simulator::new(exchanges);
    let pump = AsyncSf10::from_transport(transport).await.unwrap();
    (pump, simulator)
}

#[tokio::test]
async fn handshake_and_every_control_track_only_acknowledged_requests() {
    let (mut pump, simulator) = connected(vec![
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
    ])
    .await;
    assert_eq!(pump.firmware_version(), "SF10 v1.8");
    assert_eq!(*pump.state(), Default::default());
    pump.start().await.unwrap();
    assert_eq!(pump.state().running, Some(true));
    pump.stop().await.unwrap();
    assert_eq!(pump.state().running, Some(false));
    for valve in [Valve::A, Valve::B] {
        pump.set_valve(valve).await.unwrap();
        assert_eq!(pump.state().valve, Some(valve));
    }
    for mode in [
        Mode::Flow,
        Mode::Regulator,
        Mode::Dose,
        Mode::Gas,
        Mode::Ramp,
    ] {
        pump.set_mode(mode).await.unwrap();
        assert_eq!(pump.state().mode, Some(mode));
    }
    pump.set_flow_rate(1.2344).await.unwrap();
    pump.set_gas_flow_rate(50.12345).await.unwrap();
    pump.set_regulator_pressure(4.12345).await.unwrap();
    pump.set_dose_volume(20.12345).await.unwrap();
    pump.set_ramp(0.5, 2.0, 10.0).await.unwrap();
    assert_eq!(pump.state().flow_rate, Some(1.234));
    assert_eq!(pump.state().gas_flow_rate, Some(50.12345));
    assert_eq!(pump.state().regulator_pressure, Some(4.12345));
    assert_eq!(pump.state().dose_volume, Some(20.12345));
    assert_eq!(pump.state().ramp_initial_rate, Some(0.5));
    assert_eq!(pump.state().ramp_final_rate, Some(2.0));
    assert_eq!(pump.state().ramp_time, Some(10.0));
    simulator.finish().await;
}

#[tokio::test]
async fn safe_connect_initializes_all_settings_and_stops_on_rejected_command() {
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
    let (transport, simulator) = Simulator::new(exchanges);
    let options = ConnectionOptions {
        safe_connect: true,
        ..Default::default()
    };
    let pump = AsyncSf10::from_transport_with_options(transport, options)
        .await
        .unwrap();
    assert_eq!(pump.state().running, Some(false));
    assert_eq!(pump.state().valve, Some(Valve::A));
    assert_eq!(pump.state().mode, Some(Mode::Flow));
    assert_eq!(pump.state().flow_rate, Some(1.0));
    assert_eq!(pump.state().gas_flow_rate, Some(1.0));
    assert_eq!(pump.state().regulator_pressure, Some(1.0));
    assert_eq!(pump.state().dose_volume, Some(1.0));
    assert_eq!(pump.state().ramp_initial_rate, Some(0.5));
    assert_eq!(pump.state().ramp_final_rate, Some(1.0));
    assert_eq!(pump.state().ramp_time, Some(1.0));
    simulator.finish().await;

    let mut exchanges = handshake();
    exchanges.push(Exchange::reply("STOP", b"ERROR\r"));
    let (transport, simulator) = Simulator::new(exchanges);
    assert!(matches!(
        AsyncSf10::from_transport_with_options(transport, options).await,
        Err(VapourtecError::UnexpectedResponse { command, .. }) if command == "STOP"
    ));
    let mut server = simulator.worker.await.unwrap();
    assert_eq!(server.read(&mut [0]).await.unwrap(), 0);
}

#[tokio::test]
async fn remote_enable_requires_exact_ack_and_status() {
    for exchanges in [
        vec![Exchange::reply("REMOTEEN vtec 1", b"ERROR\r")],
        vec![
            Exchange::ok("REMOTEEN vtec 1"),
            Exchange::reply("GV", b"\r"),
        ],
        vec![
            Exchange::ok("REMOTEEN vtec 1"),
            Exchange::reply("GV", b"SF10 v1.8\r"),
            Exchange::reply("REMOTEEN", b"Remote Control is disabled\r"),
        ],
    ] {
        let (transport, simulator) = Simulator::new(exchanges);
        assert!(matches!(
            AsyncSf10::from_transport(transport).await,
            Err(VapourtecError::UnexpectedResponse { .. })
        ));
        let mut server = simulator.worker.await.unwrap();
        assert_eq!(server.read(&mut [0]).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn decimal_rounding_matches_synchronous_interface() {
    let (mut pump, simulator) = connected(vec![
        Exchange::ok("SETFLOW 1.062"),
        Exchange::ok("SETFLOW 1.188"),
        Exchange::ok("SETFLOW 2.0"),
        Exchange::ok("SETFLOW -1.0"),
    ])
    .await;
    for (value, expected) in [
        (1.0625, 1.062),
        (1.1875, 1.188),
        (1.9999, 2.0),
        (-1.0, -1.0),
    ] {
        pump.set_flow_rate(value).await.unwrap();
        assert_eq!(pump.state().flow_rate, Some(expected));
    }
    simulator.finish().await;
}

#[tokio::test]
async fn invalid_numbers_do_not_write_or_change_state() {
    let (mut pump, simulator) =
        connected(vec![Exchange::ok("SETFLOW 2.0"), Exchange::ok("STOP")]).await;
    pump.set_flow_rate(2.0).await.unwrap();
    let previous = pump.state().clone();
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for result in [
            pump.set_flow_rate(value).await,
            pump.set_gas_flow_rate(value).await,
            pump.set_regulator_pressure(value).await,
            pump.set_dose_volume(value).await,
            pump.set_ramp(value, 1.0, 1.0).await,
            pump.set_ramp(1.0, value, 1.0).await,
            pump.set_ramp(1.0, 1.0, value).await,
        ] {
            assert!(matches!(
                result,
                Err(VapourtecError::InvalidParameter { .. })
            ));
        }
    }
    assert_eq!(*pump.state(), previous);
    pump.stop().await.unwrap();
    simulator.finish().await;
}

#[tokio::test]
async fn rejected_response_preserves_prior_state_and_transport() {
    let (mut pump, simulator) = connected(vec![
        Exchange::ok("SETFLOW 2.0"),
        Exchange::reply("SETFLOW 3.0", b"ERROR\r"),
        Exchange::ok("STOP"),
    ])
    .await;
    pump.set_flow_rate(2.0).await.unwrap();
    assert!(matches!(
        pump.set_flow_rate(3.0).await,
        Err(VapourtecError::UnexpectedResponse { command, response })
            if command == "SETFLOW 3.0" && response == "ERROR"
    ));
    assert_eq!(pump.state().flow_rate, Some(2.0));
    assert!(pump.is_connected());
    pump.stop().await.unwrap();
    simulator.finish().await;
}

#[tokio::test]
async fn fragmented_replies_allow_exterior_lf_and_crlf() {
    let (transport, simulator) = Simulator::new(vec![
        Exchange::reply("REMOTEEN vtec 1", b"\nOK\r\n"),
        Exchange::reply("GV", b"SF10 v1.8\n\r"),
        Exchange::reply("REMOTEEN", b"\nRemote Control is enabled\r\n"),
        Exchange::reply("START", b"\nOK\n\r"),
    ]);
    let mut pump = AsyncSf10::from_transport(transport).await.unwrap();
    assert_eq!(pump.firmware_version(), "SF10 v1.8");
    pump.start().await.unwrap();
    assert_eq!(pump.state().running, Some(true));
    simulator.finish().await;
}

#[tokio::test(start_paused = true)]
async fn partial_reply_timeout_disconnects_and_clears_state() {
    let mut exchanges = handshake();
    exchanges.extend([Exchange::ok("SETFLOW 2.0"), Exchange::reply("START", b"O")]);
    let (transport, simulator) = Simulator::new(exchanges);
    let mut pump = AsyncSf10::from_transport_with_options(
        transport,
        ConnectionOptions {
            timeout: Duration::from_millis(50),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    pump.set_flow_rate(2.0).await.unwrap();
    assert!(matches!(
        pump.start().await,
        Err(VapourtecError::Io(error)) if error.kind() == io::ErrorKind::TimedOut
    ));
    assert!(!pump.is_connected());
    assert_eq!(*pump.state(), Default::default());
    assert!(matches!(
        pump.stop().await,
        Err(VapourtecError::NotConnected)
    ));
    let mut server = simulator.worker.await.unwrap();
    assert_eq!(server.read(&mut [0]).await.unwrap(), 0);
}

#[tokio::test(start_paused = true)]
async fn whole_exchange_timeout_covers_a_stalled_write() {
    let (mut pump, simulator) = connected(vec![Exchange::ok("SETFLOW 2.0")]).await;
    pump.set_flow_rate(2.0).await.unwrap();
    let mut server = simulator.worker.await.unwrap();
    // Stop reading from the peer. The 20-byte command cannot fit in the
    // 16-byte duplex buffer, so write_all stalls before sending its terminator.
    assert!(matches!(
        pump.set_ramp(1.0, 2.0, 3.0).await,
        Err(VapourtecError::Io(error)) if error.kind() == io::ErrorKind::TimedOut
    ));
    assert!(!pump.is_connected());
    assert_eq!(*pump.state(), Default::default());
    assert!(matches!(
        pump.stop().await,
        Err(VapourtecError::NotConnected)
    ));
    let mut written = Vec::new();
    server.read_to_end(&mut written).await.unwrap();
    assert_eq!(written, b"SETRAMP 1.0 2.0 ");
}

#[tokio::test]
async fn cancellation_after_writing_drops_transport_and_invalidates_state() {
    let (mut pump, mut simulator) = connected(vec![
        Exchange::ok("SETFLOW 2.0"),
        Exchange::reply("START", b"O"),
    ])
    .await;
    pump.set_flow_rate(2.0).await.unwrap();
    for expected in ["REMOTEEN vtec 1", "GV", "REMOTEEN", "SETFLOW 2.0"] {
        assert_eq!(simulator.commands.recv().await.unwrap(), expected);
    }
    {
        let command = pump.start();
        tokio::pin!(command);
        tokio::select! {
            result = &mut command => panic!("command completed before cancellation: {result:?}"),
            received = simulator.commands.recv() => assert_eq!(received.unwrap(), "START"),
        }
    }
    assert!(!pump.is_connected());
    assert_eq!(*pump.state(), Default::default());
    assert!(matches!(
        pump.stop().await,
        Err(VapourtecError::NotConnected)
    ));
    let mut server = simulator.worker.await.unwrap();
    assert_eq!(server.read(&mut [0]).await.unwrap(), 0);
}

#[tokio::test]
async fn malformed_and_oversized_responses_invalidate_connection() {
    for (response, oversized) in [(vec![0xff, b'\r'], false), (vec![b'x'; 1025], true)] {
        let (mut pump, simulator) = connected(vec![Exchange::reply("START", response)]).await;
        let result = pump.start().await;
        if oversized {
            assert!(matches!(
                result,
                Err(VapourtecError::ResponseTooLong { limit: 1024 })
            ));
        } else {
            assert!(matches!(result, Err(VapourtecError::InvalidResponse(_))));
        }
        assert!(!pump.is_connected());
        assert_eq!(*pump.state(), Default::default());
        simulator.finish().await;
    }
}

#[tokio::test]
async fn eof_disconnects_and_transport_ownership_is_explicit() {
    let (mut pump, simulator) = connected(vec![Exchange::eof("START")]).await;
    assert!(matches!(
        pump.start().await,
        Err(VapourtecError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof
    ));
    assert!(pump.into_transport().is_none());
    simulator.finish().await;

    let (mut pump, simulator) = connected(vec![Exchange::ok("START")]).await;
    pump.start().await.unwrap();
    pump.disconnect();
    pump.disconnect();
    assert_eq!(*pump.state(), Default::default());
    let mut server = simulator.worker.await.unwrap();
    assert_eq!(server.read(&mut [0]).await.unwrap(), 0);

    let (pump, simulator) = connected(vec![]).await;
    let transport = pump.into_transport().unwrap();
    drop(transport);
    let mut server = simulator.worker.await.unwrap();
    assert_eq!(server.read(&mut [0]).await.unwrap(), 0);
}
