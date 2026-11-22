# Installation

!!! warning
    We use [semantic versioning](https://semver.org/).
    
    This means
    
    * We believe the code is stable enough to be used for production
    * everything before version 1.0.0 could be changed at any time without prior notice.
      You should expect pinouts to change, features to dissapear, etc. with any upgrade
    * We maintain a changelog that will notify you of significant changes
    * Starting with version 1.0.0 the layout and features can be considered stable in a sense
      that they might be improved, but in a backward compatible way


The latest version of vTally is [{{ latest_version }}]({{ github_link }}/releases/tag/{{ latest_version }}).

The hub is a single program. You do not need to install anything else to run it.
Download the archive for your computer from the [release page]({{ github_link }}/releases/tag/{{ latest_version }}),
extract it, and start `vtally-hub` (`vtally-hub.exe` on Windows) from the extracted directory. Then point your
browser to <http://localhost:3000> to see the hub.

| Computer | Archive |
| --- | --- |
| Windows (x86_64) | `vtally-hub-{{ latest_version }}-x86_64-pc-windows-msvc.zip` |
| Windows (ARM) | `vtally-hub-{{ latest_version }}-aarch64-pc-windows-msvc.zip` |
| macOS (Apple silicon) | `vtally-hub-{{ latest_version }}-aarch64-apple-darwin.tar.gz` |
| macOS (Intel) | `vtally-hub-{{ latest_version }}-x86_64-apple-darwin.tar.gz` |
| Linux (x86_64) | `vtally-hub-{{ latest_version }}-x86_64-unknown-linux-gnu.tar.gz` |
| Linux (ARM64, e.g. Raspberry Pi) | `vtally-hub-{{ latest_version }}-aarch64-unknown-linux-gnu.tar.gz` |

## Windows

Extract the zip file and double-click `vtally-hub.exe`. Windows Defender might complain that it can not
verify the source. Just be stubborn. A terminal window opens; the hub runs as long as it is open.

To start the hub automatically, hit `WIN` + `R` on your keyboard, type `shell:Startup` and put a shortcut
to `vtally-hub.exe` in this directory.

## macOS

````bash
tar -xzf vtally-hub-{{ latest_version }}-aarch64-apple-darwin.tar.gz
cd vtally-hub-{{ latest_version }}-aarch64-apple-darwin
./vtally-hub
````

macOS might refuse to open a program of an unknown developer. Open the "Privacy & Security" settings and allow it.

## Linux

````bash
tar -xzf vtally-hub-{{ latest_version }}-x86_64-unknown-linux-gnu.tar.gz
cd vtally-hub-{{ latest_version }}-x86_64-unknown-linux-gnu
./vtally-hub
````

Quit by tapping `CTRL` + `c` on your keyboard in the terminal window.

## Options

Run `vtally-hub --help` to see all options, for instance `--port` to change the port of the web interface
(default {{ hub_default_port }}) or `--tally-port` for the port the Tallies talk to (default {{ tally_default_port }}).
