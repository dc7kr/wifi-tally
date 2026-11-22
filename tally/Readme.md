# vTally Light

This is the firmware for the Tally Light. It is a [PlatformIO](https://platformio.org) project
using the Arduino framework and runs on ESP8266 (NodeMCU) and ESP32.

[Full documentation at wifi-tally.github.io](https://wifi-tally.github.io/)

## Build & flash

```sh
cp data/tally-settings.ini.example data/tally-settings.ini   # then edit it

pio run -e nodemcuv2 -t upload      # or: -e esp32dev
pio run -e nodemcuv2 -t uploadfs    # uploads the settings file (LittleFS)
pio device monitor
```

## Tests

Parsing, protocol, log buffer and LED math live in `lib/tally_core` without any Arduino
dependency and are tested on the host:

```sh
pio test -e native
```

## Layout

- `lib/tally_core/` – platform independent logic (settings, hub protocol, log buffer, LED math)
- `src/` – hardware glue: `Leds` (PWM + WS2812), `WifiManager`, `TallyLink` (UDP), `Log`
- `src/Board.h` – pin assignments per chip family (the ESP8266 pins match the previous NodeMCU wiring)

## Adding another board

Copy an `[env:...]` section in `platformio.ini` and change `board`. If it is neither ESP8266 nor ESP32,
add pins to `src/Board.h`.
