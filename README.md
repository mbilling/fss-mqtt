# fss-mqtt

A fast, keyboard-driven MQTT explorer for the terminal. Subscribe to a broker and get a
live topic tree you narrow down just by typing.

```
 ✻ fss-mqtt  mqtt://localhost:1883  ● connected                      9 topics  21 msgs  412/s
╭─ Topics ──────────────────────────────╮╭─ v1/plant/line1/temp ─────────────────────────╮
│   ▾ v1                             19 ││ 3 msgs · last now · qos 1                     │
│     ▾ plant                        18 ││                                               │
│       ▾ line1                       6 ││ Messages                                      │
│         • status = RUNNING          3 ││ › 17:59:19.363   49 B  {"t":22.5,"ok":true,…  │
│ ❯       • temp = {"t":22.5,"ok":…   3 ││   17:59:18.912   49 B  {"t":21.5,"ok":true,…  │
│     ▸ camera (1)                    1 ││                                               │
│                                       ││ Properties                                    │
│                                       ││   content-type  application/json              │
│                                       ││   site          oslo                          │
│                                       ││ Payload · 49 B                                │
│                                       ││   {"t":22.5,"ok":true,"tags":["a","b"],"no…   │
╰───────────────────────────────────────╯╰───────────────────────────────────────────────╯
╭───────────────────────────────────────────────────────────────────────────────────────╮
│ > v1/+/line1                                                       pattern · 4 topics │
╰───────────────────────────────────────────────────────────────────────────────────────╯
  ↑↓ move  ·  ←→ collapse/expand  ·  ⏎ open  ·  esc clear filter  ·  ^w up a level  ·  ^c quit
```

## Install

Pre-built binaries for Linux, macOS and Windows are on the GitHub releases page.

```sh
# Debian / Ubuntu
sudo apt install ./fss-mqtt_<version>-1_amd64.deb
# Fedora / RHEL / openSUSE
sudo dnf install ./fss-mqtt-<version>-1.x86_64.rpm
# Homebrew (macOS, Linux)
brew install <owner>/tap/fss-mqtt
# Windows
scoop bucket add fss-mqtt https://github.com/<owner>/scoop-bucket && scoop install fss-mqtt
winget install <Publisher>.FssMqtt
```

The Linux binaries are static and run on any distribution. Package-manager channels become available
once they're set up (see [RELEASING.md](RELEASING.md)).

From source (Rust 1.88+):

```sh
cargo build --release        # → target/release/fss-mqtt (~1.1 MB)
cargo install --path .       # or install it to ~/.cargo/bin
```

## Usage

```sh
fss-mqtt broker.example.com                # connect (and save the connection)
fss-mqtt mqtts://broker:8883 -u me         # TLS (system trust store); asks for the password
fss-mqtt broker:8883 --cafile ca.pem       # TLS with your own CA
fss-mqtt broker -t 'v1/#' -t '$SYS/#'      # several subscriptions
fss-mqtt                                   # connect the saved connections marked autoconnect
fss-mqtt -c local -c staging               # connect these saved connections
```

Connects with MQTT v5 (so user properties are visible) and reconnects automatically.
TCP and TLS only; WebSocket brokers (`ws://`, `wss://`) are not supported.
`FSS_MQTT_BROKER`, `FSS_MQTT_USERNAME`, `FSS_MQTT_PASSWORD` and `FSS_MQTT_CAFILE` are read from the environment.
With no saved connections and no broker given, it connects to `mqtt://localhost:1883` (not saved).

## Connections

Several brokers can be connected at once. Each is a top-level branch in the topic tree and keeps its
own topics and history while you connect, disconnect or pause its subscriptions. Filters match
below the broker level, so `devices/#` shows matches from every broker.

Connections are kept in a small file you can edit:
`~/.config/fss-mqtt/connections.toml` (Linux, macOS; `$XDG_CONFIG_HOME` is honoured) or
`%APPDATA%\fss-mqtt\connections.toml` (Windows); `--config` picks another file.

```toml
[[connection]]
name         = "local"
url          = "mqtts://127.0.0.1:8883"
cafile       = "~/secrets/ca.pem"          # also: cert, key, insecure = true
username     = "backend"
password_env = "BROKER_PW"                 # read from this environment variable
client_id    = "fss-mqtt-ci"               # optional; random by default
qos          = 1                           # optional; 0 by default
topics       = ["devices/+/up/#", "$SYS/#"]
paused       = ["$SYS/#"]                  # listed, but not subscribed
autoconnect  = true                        # connect when started without -c or a broker
```

- **Adding:** connect from the command line (the entry is added or updated, named `user@host:port`
  or `--name`; `--no-save` skips this), or add a block to the file. `--cafile`, `--cert` and
  `--key` are saved as absolute paths, so the entry works from any directory; in the file you can
  also write `~/…`.
- **Passwords are never written.** Set `password_env`, or fss-mqtt asks when it connects and keeps
  the password in memory only. `FSS_MQTT_PASSWORD` is saved as `password_env`; `-P` is not.
- **Saving** rewrites only that connection's block, so your comments elsewhere in the file stay.
  Pausing or resuming a subscription in the panel is saved the same way.

The panel at the top lists each connection with its subscriptions. `tab` moves between the panel,
the topic tree and the message list; in the panel, `space` or `⏎` connects or disconnects a
connection, or turns a subscription on or off (subscribed and unsubscribed live).

| Flag | Default | |
|---|---|---|
| `-c, --connection` | | saved connection to connect (repeatable) |
| `--name` | `user@host:port` | name to save the broker under |
| `--no-save` | | don't add the broker to the connections file |
| `--config` | see above | connections file |
| `-t, --topic` | `#` | topic filter to subscribe to (repeatable) |
| `--history` | `10` | messages kept per topic |
| `--max-payload` | `4KiB` | payload bytes kept per message; the rest is discarded on arrival |
| `--inline` | `64` | payloads up to this size are shown inline in the tree; larger ones show only their size |
| `--preview` | `50` | payload bytes shown in the detail pane |
| `-q, --qos` | `0` | subscription QoS |
| `--cafile` | | PEM file of CA certificates to trust (implies TLS) |
| `--cert`, `--key` | | client certificate and key for mutual TLS |
| `--insecure` | | skip TLS certificate verification |

## Filtering

Just type — the tree updates on every keystroke and the cursor jumps to the first match.

- **Pattern** (input contains `/`, `+` or `#`): MQTT-style. `v1/#`, `v1/+/temp`, `+/+/status`.
  The last level is a case-insensitive *prefix*, so `v1/pl` already shows `v1/plant`, and
  everything below a match is included.
- **Search** (anything else): case-insensitive substring over the full topic path; matches are
  underlined.

`backspace` deletes a character, `ctrl+w` deletes back to the previous level, `esc`/`ctrl+u`
clears. Clearing keeps the tree open at the topic you were on.

## Keys

| Where | Key | |
|---|---|---|
| anywhere | `tab` / `shift+tab` | next / previous pane: connections, topics, messages |
| connections | `↑ ↓` | move |
| connections | `space` `⏎` | connect / disconnect, or subscription on / off |
| tree | `↑ ↓ pgup pgdn home end` | move |
| tree | `→` | expand; on a topic with messages, enter its message list |
| tree | `←` | collapse, or jump to parent |
| tree | `⏎` | open the latest message full screen |
| messages | `↑ ↓` | pick one of the last 10 messages (properties and preview follow) |
| messages | `⏎` | open it full screen |
| messages | `← esc` | back to the tree |
| full screen | `↑ ↓ pgup pgdn` | scroll |
| full screen | `← →` | older / newer message |
| full screen | `tab` | cycle pretty JSON / raw / hex |
| full screen | `esc q ⏎` | close |
| anywhere | `ctrl+c` | quit |

A green `•`/`▸` means the topic (or something under it) received a message in the last 1.5 s.

Colours follow `$COLORFGBG` for light terminals (e.g. `export COLORFGBG='0;15'`) and default to dark.

## Performance notes

Messages are written straight into an in-memory tree; the UI redraws at ~15 fps rather than per
message, and only the visible rows are rendered. Large payloads are never rendered in the tree. Each
topic keeps its last `--history` messages (10), and each message keeps only the first `--max-payload`
bytes (4 KiB); the rest is discarded as it arrives, so a topic costs at most about 40 KiB of payload
however large or frequent its messages are. Filtering 100k topics takes a few milliseconds.

The MQTT client (`src/mqtt.rs`) is a small MQTT v5 subscriber on blocking I/O with one thread and
no async runtime, which keeps the binary around 1 MB. TLS uses rustls.

## Development

CI (`.github/workflows/ci.yml`) runs fmt, clippy and tests on Linux, macOS and Windows, checks the
minimum Rust version, runs the end-to-end test against mosquitto (plain and TLS), and guards the
release binary size.

```sh
cargo test
# end-to-end: starts mosquitto on 127.0.0.1:18830 (plain) and localhost:18883 (TLS)
scripts/e2e-brokers.sh /tmp/fss-brokers
FSS_TEST_CA=/tmp/fss-brokers/ca.pem cargo test --release e2e -- --ignored --nocapture
# filter benchmark over 100k topics:
cargo test --release bench_filter -- --ignored --nocapture
```
