
## Features

| Feature | Provides |
| --- | --- |
| Default (empty) | Blocking `Sf10<T>` client |
| `async` | Tokio-based `AsyncSf10<T>` client |
| `serialport` | `open_serial_port` and concrete `NativeSerialPort` type |
| `tokio-serial` | `open_tokio_serial_port`; also enables `async` |

## Blocking serial example


```rust,no_run
use std::time::Duration;
use vapourtec::{open_serial_port, ConnectionOptions, Mode, Sf10, Valve};

fn main() -> vapourtec::Result<()> {
    // Use your pump's port, e.g. COM6 or /dev/ttyUSB0.
    let transport = open_serial_port("COM6", Duration::from_secs(1))?;
    let mut pump = Sf10::from_transport_with_options(
        transport,
        ConnectionOptions {
            safe_connect: true,
            ..Default::default()
        },
    )?;

    pump.set_valve(Valve::A)?;
    pump.set_mode(Mode::Flow)?;
    pump.set_flow_rate(2.5)?;
    pump.start()?;
    // Perform the application's work here.
    pump.stop()?;
    pump.disconnect();
    Ok(())
}
```

`open_serial_port(port, timeout)` returns a concrete `serialport::TTYPort` on Unix or `serialport::COMPort` on Windows through the `NativeSerialPort` alias. It configures 9600 baud, 8N1, no flow control, the supplied I/O timeout, and clears stale input. It sends no pump commands.

## Async serial example


```rust,no_run
use vapourtec::{open_tokio_serial_port, AsyncSf10, ConnectionOptions};

#[tokio::main(flavor = "current_thread")]
async fn main() -> vapourtec::Result<()> {
    let transport = open_tokio_serial_port("COM6")?;
    let mut pump = AsyncSf10::from_transport_with_options(
        transport,
        ConnectionOptions {
            safe_connect: true,
            ..Default::default()
        },
    )
    .await?;

    pump.set_flow_rate(2.5).await?;
    pump.start().await?;
    // Perform the application's work here.
    pump.stop().await?;
    pump.disconnect();
    Ok(())
}
```

`open_tokio_serial_port(port)` returns a `tokio_serial::SerialStream` configured for 9600 baud, 8N1, no flow control, with stale input cleared. Call this synchronous constructor inside a Tokio runtime with its I/O driver enabled. Async commands also require the runtime's timer driver for their deadlines.

## Transports and connection options

If you already own a transport, pass it directly to `Sf10::from_transport(transport)` or `AsyncSf10::from_transport(transport).await`. To customize initialization, use the corresponding `from_transport_with_options(transport, options)` constructor. Commands and constructors are async on `AsyncSf10`; state access, `disconnect()`, and `into_transport()` are synchronous on both clients.

`ConnectionOptions` defaults to `safe_connect: false` and `timeout: Duration::from_secs(1)`. The blocking client cannot configure an arbitrary transport's timeout, so configure it yourself or pass it to `open_serial_port`. The async client applies `timeout` to the complete command exchange, including writing, flushing, and reading the response.

Both constructors enable remote control with `REMOTEEN vtec 1`, query firmware using `GV`, and verify the `REMOTEEN` response. Firmware 1.8 is the original documented prerequisite. `firmware_version()` returns the firmware reply without enforcing a version format.

With `safe_connect: true`, initialization also sends commands to stop the pump, select valve A and FLOW mode, set liquid flow to 1.0 mL/min, gas flow to 1.0 ssc/min, regulator pressure to 1.0 bar, dose volume to 1.0 mL, and a ramp from 0.5 to 1.0 mL/min over one minute. Each command must be acknowledged before initialization continues.

The clients do not reopen transports. To reconnect, create a new transport using your chosen driver or helper, then pass it to a new client. `into_transport()` consumes a client and returns its transport if still connected.

## Pump operations and state

Both clients expose these operations; async calls require `.await`. Each returns `Result<()>`.

| Operation | Meaning |
| --- | --- |
| `start()`, `stop()` | Start or stop pumping |
| `set_valve(Valve::A)` / `set_valve(Valve::B)` | Select the inlet valve |
| `set_mode(mode)` | Select `Mode::Flow`, `Regulator`, `Dose`, `Gas`, or `Ramp` |
| `set_flow_rate(value)` | Liquid flow in mL/min, shared by FLOW and DOSE modes |
| `set_gas_flow_rate(value)` | Gas flow in ssc/min |
| `set_regulator_pressure(value)` | Pressure in bar |
| `set_dose_volume(value)` | Dose volume in mL |
| `set_ramp(initial, final_rate, minutes)` | Ramp flow rates in mL/min and duration in minutes |

Numeric parameters must be finite. Liquid flow is rounded to three decimal places with ties to even. Use the pump's documented operating ranges; the library does not validate physical limits. The pump may acknowledge an out-of-range value with `OK` while applying a default, so `state()` records **last acknowledged requests**, not physical readback. Each field is an `Option` and remains `None` until acknowledged.

