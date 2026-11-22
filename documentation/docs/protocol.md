# Protocol

This page specifies how the Hub and a WiFi Tally talk to each other. You only need it if you want
to build your own Tally (on other hardware or in another language) or write a custom Hub.

The reference implementations are the [Tally firmware](https://github.com/wifi-tally/wifi-tally/tree/master/tally)
(`lib/tally_core/Protocol.cpp`, `src/TallyLink.cpp`) and the Hub
(`hub-rs/src/command.rs`).

## Transport

* Plain text messages in single **UDP datagrams**. One datagram carries exactly one message.
* All messages are ASCII. Leading and trailing whitespace (e.g. a trailing `\n`) is ignored by the receiver.
* The Hub listens on a UDP port (default `{{ tally_default_port }}`). The Tally learns the address from
  its `hub.ip` and `hub.port` settings.
* The Tally sends from a socket that it also listens on (default port `{{ tally_default_port }}`). The Hub
  does **not** need to be configured with the address of a Tally: it answers to the source address
  and source port of the last datagram it received from that Tally. Tallies therefore must
  send from the same socket they receive on.
* There is no encryption, authentication or acknowledgement. Lost datagrams are compensated by
  sending everything repeatedly (see [Timing](#timing)).

A Tally is identified by its **name**, which has to be unique in the network. Names are at most 26
characters long. Since the name is embedded in a quoted string, it should not contain `"`.

## Tally → Hub

### `tally-ho`

Announces that the Tally exists and is alive. It is sent immediately after the Tally got an IP and then
repeatedly (once per second).

    tally-ho "<name>"

Example:

    tally-ho "Doe"

When the Hub receives it for the first time, it creates the Tally. Every further message
updates the address the Hub sends state commands to.

### `log`

Sends a log message to be displayed in the Hub.

    log "<name>" <severity> "<message>"

| field | description |
| --- | --- |
| `name` | The name of the Tally, like in `tally-ho` |
| `severity` | `INFO`, `WARN` or `ERROR`. Anything else is treated as `ERROR` by the Hub. |
| `message` | Free text. The firmware truncates it to 80 characters. |

Example:

    log "Doe" ERROR "Got disconnected from MyWifi. Reason 2"

A `log` message also counts as a sign of life, just like `tally-ho`. Tallies buffer a small number of
logs while no connection to the Hub exists and send them one by one once it is established.
The firmware keeps at most 10 logs; when full, the oldest log with the lowest severity is dropped first.

## Hub → Tally

The Hub continuously tells each connected Tally which colors to show. A message is either a
[static color](#static-color) or a [flashing color](#flashing-color).

The Tally has two lights, which can be driven independently:

* **operator light** (`O`): faces the camera operator. Shows preview, program and idle states.
* **stage light** (`S`): faces the stage. Usually only shows program.

### Static color

    O<rrr>/<ggg>/<bbb> S<rrr>/<ggg>/<bbb>

Each component is a zero padded, 3 digit **decimal** brightness from `000` to `255`. The message is always
exactly **25 characters** long (ignoring surrounding whitespace).

| characters | meaning |
| --- | --- |
| `O` | literal, marks the operator light |
| `rrr/ggg/bbb` | red, green and blue of the operator light |
| ` S` | a space, then a literal `S`, marks the stage light |
| `rrr/ggg/bbb` | red, green and blue of the stage light |

Example: operator light red, stage light off

    O255/000/000 S000/000/000

### Flashing color

A static color followed by a flash pattern and a step duration. The message is exactly **34 characters** long.

    O<rrr>/<ggg>/<bbb> S<rrr>/<ggg>/<bbb> 0x<pp> <ddd>

| field | description |
| --- | --- |
| `0x<pp>` | The pattern as a 2 digit **hexadecimal** byte (upper or lower case), prefixed with `0x`. |
| `<ddd>` | The duration of one step in **milliseconds** as a 3 digit decimal (`001`–`999`). |

The pattern has 8 steps. The most significant bit is the first step. A `1` means the light is
*bright* (the given color), a `0` means *dark*. After the last step the pattern starts over.
Every `<ddd>` milliseconds the Tally advances one step.

| pattern | steps |
| --- | --- |
| `0xAA` | `1 0 1 0 1 0 1 0`: fast blinking |
| `0x80` | `1 0 0 0 0 0 0 0`: short flash, then a long pause |

A *dark* step does not turn the lights off, but dims them to **30 %** of the given brightness
(rounded up), so the Tally never looks broken:

    dark = ceil(brightness * 0.3)

If a Tally receives the same flashing message again while it is already running that pattern, it must
keep it running without restarting, as the Hub sends the message repeatedly and a restart would
destroy the rhythm.

Example: operator light blinking green, stage light off, 300 ms per step

    O000/255/000 S000/000/000 0xAA 300

### Invalid messages

Messages of any other length, with different separators, non-digit characters in numbers
or values above `255` must be ignored. The firmware logs a `WARN` for them.

### State and color mapping

The colors are chosen by the Hub according to the state of the Tally (and the color scheme and brightness
configured there). The states and the messages they use:

| state | operator light | stage light | pattern |
| --- | --- | --- | --- |
| program (on air) | program color | program color | static |
| preview | preview color | preview color, or black if the stage light does not show preview | static |
| idle (not on program or preview) | idle color, or black | black | static |
| highlight (identify this Tally) | highlight color | highlight color | `0xAA`, 125 ms |
| unknown (video mixer not connected) | unknown color | black | `0x80`, 250 ms |

## Timing

| what | value |
| --- | --- |
| Tally sends `tally-ho` | every 1 s while WiFi is up |
| Hub sends the current state | 10 times per second to every active Tally (default, configurable in the Hub) |
| Tally considers the Hub lost | no datagram from the Hub for more than 3 s |
| Hub marks a Tally as *missing* | no message from it for 3 s |
| Hub marks a Tally as *disconnected* | no message from it for 30 s |

While the Tally considers the Hub lost, it shows the "waiting for server" blink pattern (see
[Troubleshooting](troubleshooting.md)) and keeps sending `tally-ho`, so the connection recovers
as soon as the Hub is back.

## Sample session

    Tally → Hub   tally-ho "Doe"
    Hub → Tally   O000/000/000 S000/000/000
    Hub → Tally   O000/000/000 S000/000/000
    ...
    Tally → Hub   tally-ho "Doe"
    Hub → Tally   O000/255/000 S255/000/000          (channel went to program)
    Tally → Hub   log "Doe" WARN "invalid package: foo"
