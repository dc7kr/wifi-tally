# vTally Hub (Rust)

A rewrite of the [vTally hub](../hub) in Rust. It is a drop-in replacement for the
Node.js backend: it serves the existing React frontend, speaks the same socket.io
protocol, talks the same UDP protocol to the tallies and reads and writes the same
configuration file (`~/.wifi-tally.json`).

The result is a single ~7 MB executable without a Node.js runtime.

## Status

| Area | State |
| --- | --- |
| Web server + socket.io (v2 protocol, as used by the frontend) | done |
| UDP tally driver (tally-ho, logs, missing/disconnected detection, keep alives) | done |
| Web tallies (browser as a tally) | done |
| Tally management (patch, highlight, remove, settings, defaults, logs) | done |
| Configuration file (reads files written by older hubs, keeps settings of all mixers) | done |
| `null`, `mock` (development) and `test` mixers | done |
| **vMix** connector (TCP API) | done |
| **ATEM** connector (own implementation of the UDP protocol, see below) | done – **verified against the reference library and a simulated ATEM, not yet against real hardware** |
| **OBS** connector (obs-websocket v4) | done – tested against a simulated OBS, not yet against a real one; no password support (as before) |
| **Roland V-60HD** connector (Smart Tally over HTTP) | done – tested against a simulated V-60HD, not yet against a real one |
| **Roland V-8HD** connector (MIDI via `midir`) | done – message handling is tested, the MIDI transport **not yet against a real V-8HD**. Needs ALSA (`libasound2-dev`) to build on Linux. |
| Tally flasher (NodeMCU programming from the hub) | removed from the hub – flash the firmware with PlatformIO |
| Development proxy to the React dev server | not ported – use `--frontend-dir` with a built frontend |

## Build and run

    cargo build --release
    ./target/release/vtally-hub --frontend-dir ../frontend/build

The frontend is built in `../frontend` with `npm run build`. Without `--frontend-dir`
the hub looks for a `frontend` directory next to the executable.

    --env <production|development>  development adds the simulated "mock" mixer and verbose logging
    --port <PORT>                   web interface, default 3000 (env: PORT)
    --tally-port <PORT>             UDP port of the tallies, default 7411
    --config-file <FILE>            default ~/.wifi-tally.json (env: CONFIG_FILE)
    --frontend-dir <DIR>            built web frontend (env: VTALLY_FRONTEND_DIR)
    --with-test                     scripted test mixer, no configuration file (used by the e2e test)

## Package

    ./package.sh                  # npm ci --ignore-scripts (Node 16+) + frontend build, release build, then dist/vtally-hub-<version>-<target>.tar.gz
    ./package.sh --skip-frontend  # reuse an existing ../frontend/build
    ./package.sh --target x86_64-apple-darwin --version 1.2.3   # cross compile (rustup target add first)

The archive holds the executable and a `frontend/` directory next to it, which the hub finds without
any option. Windows targets produce a `.zip`.

Log verbosity can be tuned with `RUST_LOG`, e.g. `RUST_LOG=vtally_hub=debug`.

## Tests

    cargo test        # unit tests, ported from the Jest specs of the TypeScript hub
    cargo clippy --all-targets

    # the ATEM tally logic is compared with the `atem-connection` library of the Node hub on random
    # command sequences (tests/atem_reference.rs). To regenerate tests/fixtures/atem-visible-inputs.json:
    #   npm install atem-connection@3.4.0   (in a scratch directory)
    #   NODE_PATH=<scratch>/node_modules node tests/atem-fixtures.js > tests/fixtures/atem-visible-inputs.json

    # end-to-end: starts the binary and drives it with the real socket.io v2 client and UDP
    (cd ../frontend && npm ci)   # provides socket.io-client
    cargo build && NODE_PATH=../frontend/node_modules node tests/e2e/e2e.js target/debug/vtally-hub

## Architecture

    udp.rs ───────┐                                   ┌── server.rs ── socket.io rooms ── browsers
    server.rs ────┼──►  hub.rs  (state behind one     │
    mixer/ ───────┘     Mutex, no await while locked) ─┼── persistence.rs ── ~/.wifi-tally.json
                              │  broadcast::Sender<Event>
                              └──────────────────────  └── mixer/driver.rs ── connector for the selected mixer

* `hub.rs` owns all state (configuration, tallies, current program/preview) and implements
  what the TypeScript hub spread over `TallyContainer`, `MixerCommunicator` and the tally drivers.
  Every change is announced as an `Event`.
* `server.rs` turns events into socket.io messages. A subscription (`events.<topic>.subscribe`)
  is a socket.io room, replacing the hand-written `SocketAwareEvent` bookkeeping.
* `persistence.rs` saves (debounced, atomically) when a persisted value changes, and once more on shutdown.
* `mixer/connectors.rs` is where the remaining mixer connectors go: implement `Connector`
  (`connect`/`disconnect`), report through the `Hub::notify_*` methods and register it in `create`.
  A connector starts a task in `connect` (`TaskGuard` stops it when dropped).
* `mixer/vmix.rs`: line/length framing of the TCP API (`Parser`), reconnects every second, polls the
  input names with `XML` every 5 seconds.
* `mixer/obs.rs`: `ObsState` is the pure logic (scenes embedded into live scenes, live mode); the connector
  speaks the obs-websocket v4 JSON protocol and asks for the scene list again whenever scenes change.
* `mixer/roland_v60hd.rs`: polls `/tally/<n>/status` of the eight inputs; `mixer/roland_v8hd.rs`: asks the
  V-8HD for the tally of its eight inputs with SysEx messages (`InputStatus` parses the answers).
* `mixer/atem/`: `packet.rs` is the UDP framing, `state.rs` the commands that matter for tallies
  plus the port of `listVisibleInputs` (keyers, transitions, super source, nested mix effects),
  `mod.rs` the session (hello, acknowledgements) and the reconnect logic. The hub only listens and
  never sends commands. There is no Rust ATEM library, so this replaces `atem-connection`.

## Differences to the TypeScript hub

* Roland V-8HD/V-60HD settings changes are now pushed to other open browsers (the TypeScript hub forgot to).
* Configuration is only written when something persisted really changed (the TypeScript hub compared object
  references and so saved on every tally update).
* OBS `liveMode` values other than `always`, `stream`, `record` and `streamOrRecord` are rejected.
* Unknown or malformed fields in the configuration file are logged and skipped, as before; a file that is not
  valid JSON still stops the hub from starting, so it is never overwritten.
* vMix: the answer to `XML` is read by its announced length, so XML that contains line breaks or arrives in
  several TCP segments works (the TypeScript hub split every TCP chunk at line breaks). While vMix is
  unreachable the connection error is logged once, not on every attempt.
* ATEM: tallies are only published once the ATEM has sent its complete state, not while it is still being
  transferred. "Could not connect" and "Lost connection" are logged once per outage.
* OBS: the streaming and recording state is also queried right after connecting (`GetStreamingStatus`), so the
  live mode is right when OBS was already streaming. The TypeScript hub only learned it from later events.
* Roland V-60HD: the eight inputs are polled together, once per request interval, and published when all of
  them answered. If one fails the mixer counts as disconnected until all answer again (the TypeScript hub
  polled every input on its own timer).
* Roland V-8HD: when the MIDI ports are not found (or the device is unplugged) the hub tries again every
  two seconds. The TypeScript hub gave up after the first attempt.
