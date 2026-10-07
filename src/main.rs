//! fss-mqtt — interactive terminal MQTT explorer.

mod app;
mod fmt;
mod mqtt;
mod store;
mod theme;
mod ui;
mod viewer;

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::stdout;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ratatui::crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind};
use ratatui::crossterm::execute;

const FRAME: Duration = Duration::from_millis(66);
const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "fss-mqtt — interactive MQTT explorer

Usage:
  fss-mqtt [flags] [broker]

Flags:
  -b, --broker string        broker URL (mqtt://, mqtts://) or host[:port] (default \"mqtt://localhost:1883\")
      --cafile string        PEM file of CA certificates to trust (implies TLS)
      --cert string          client certificate PEM for mutual TLS
  -i, --client-id string     client id (default fss-mqtt-<random>)
      --history int          messages kept per topic (default 10)
      --inline int           payloads up to this many bytes are shown inline in the tree (default 64)
      --insecure             skip TLS certificate verification
      --key string           client private key PEM for mutual TLS
      --max-payload string   bytes of each payload kept in memory; larger payloads are truncated (default \"64KiB\")
  -P, --password string      password
      --preview int          payload bytes shown in the detail pane (default 50)
  -q, --qos int              subscription QoS (0, 1, 2)
  -t, --topic string         topic filter to subscribe to (repeatable) (default \"#\")
  -u, --username string      username
  -v, --version              print version and exit

Environment: FSS_MQTT_BROKER, FSS_MQTT_USERNAME, FSS_MQTT_PASSWORD, FSS_MQTT_CAFILE";

struct Args {
    broker: String,
    topics: Vec<String>,
    username: Option<String>,
    password: Option<String>,
    client_id: Option<String>,
    qos: u8,
    history: usize,
    max_payload: usize,
    preview: usize,
    inline: usize,
    insecure: bool,
    ca_file: Option<String>,
    cert: Option<String>,
    key: Option<String>,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("fss-mqtt: {e}");
        std::process::exit(1);
    }
}

fn parse_args() -> Result<Option<Args>, String> {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let mut a = Args {
        broker: env("FSS_MQTT_BROKER").unwrap_or_else(|| "mqtt://localhost:1883".into()),
        topics: vec![],
        username: env("FSS_MQTT_USERNAME"),
        password: env("FSS_MQTT_PASSWORD"),
        client_id: None,
        qos: 0,
        history: 10,
        max_payload: 64 * 1024,
        preview: 50,
        inline: 64,
        insecure: false,
        ca_file: env("FSS_MQTT_CAFILE"),
        cert: None,
        key: None,
    };
    let num = |name: &str, v: String| v.parse::<usize>().map_err(|_| format!("{name}: invalid number {v:?}"));

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let (name, mut inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        let mut val = || {
            inline
                .take()
                .or_else(|| it.next())
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match name.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(None);
            }
            "-v" | "--version" => {
                println!("fss-mqtt {VERSION}");
                return Ok(None);
            }
            "-b" | "--broker" => a.broker = val()?,
            "-t" | "--topic" => a.topics.push(val()?),
            "-u" | "--username" => a.username = Some(val()?),
            "-P" | "--password" => a.password = Some(val()?),
            "-i" | "--client-id" => a.client_id = Some(val()?),
            "-q" | "--qos" => {
                a.qos = match val()?.as_str() {
                    "0" => 0,
                    "1" => 1,
                    "2" => 2,
                    _ => return Err("--qos must be 0, 1 or 2".into()),
                }
            }
            "--history" => a.history = num(&name, val()?)?,
            "--max-payload" => a.max_payload = parse_size(&val()?).map_err(|e| format!("--max-payload: {e}"))?,
            "--preview" => a.preview = num(&name, val()?)?,
            "--inline" => a.inline = num(&name, val()?)?,
            "--insecure" => a.insecure = true,
            "--cafile" => a.ca_file = Some(val()?),
            "--cert" => a.cert = Some(val()?),
            "--key" => a.key = Some(val()?),
            s if s.starts_with('-') && s.len() > 1 => return Err(format!("unknown flag {s}\n\n{USAGE}")),
            _ => a.broker = arg,
        }
    }
    if a.topics.is_empty() {
        a.topics.push("#".into());
    }
    Ok(Some(a))
}

fn parse_size(s: &str) -> Result<usize, String> {
    let t = s.trim().to_uppercase();
    let (num, mult) = [
        ("KIB", 1 << 10),
        ("MIB", 1 << 20),
        ("KB", 1 << 10),
        ("MB", 1 << 20),
        ("K", 1 << 10),
        ("M", 1 << 20),
        ("B", 1),
    ]
    .iter()
    .find_map(|(suf, m)| t.strip_suffix(suf).map(|n| (n.trim().to_string(), *m)))
    .unwrap_or((t.clone(), 1));
    num.parse::<usize>()
        .map(|n| n * mult)
        .map_err(|_| format!("invalid size {s:?}"))
}

struct Broker {
    host: String,
    port: u16,
    tls: bool,
    display: String,
}

/// Accepts a URL or host[:port]. A bare host uses TLS when TLS options were
/// given or the port is 8883.
fn parse_broker(s: &str, tls_wanted: bool) -> Result<Broker, String> {
    let (scheme, rest) = match s.split_once("://") {
        Some((sc, r)) => (sc.to_lowercase(), r),
        None => (
            if tls_wanted || s.ends_with(":8883") {
                "mqtts"
            } else {
                "mqtt"
            }
            .to_string(),
            s,
        ),
    };
    let tls = match scheme.as_str() {
        "mqtt" | "tcp" => false,
        "mqtts" | "ssl" | "tls" | "tcps" | "mqtt+ssl" => true,
        "ws" | "wss" => return Err("WebSocket brokers (ws://, wss://) are not supported".into()),
        other => return Err(format!("broker: unsupported scheme {other:?}")),
    };
    let hostport = rest.split('/').next().unwrap_or("");
    let hostport = hostport.rsplit_once('@').map(|(_, h)| h).unwrap_or(hostport);
    let (host, port) = if let Some(r) = hostport.strip_prefix('[') {
        // [ipv6]:port
        let (h, p) = r.split_once(']').ok_or("broker: bad IPv6 address")?;
        (h.to_string(), p.strip_prefix(':').map(str::to_string))
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), Some(p.to_string())),
            None => (hostport.to_string(), None),
        }
    };
    if host.is_empty() {
        return Err(format!("broker: missing host in {s:?}"));
    }
    let port = match port {
        Some(p) => p.parse().map_err(|_| format!("broker: bad port {p:?}"))?,
        None if tls => 8883,
        None => 1883,
    };
    let shown_host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.clone()
    };
    Ok(Broker {
        display: format!("{}://{shown_host}:{port}", if tls { "mqtts" } else { "mqtt" }),
        host,
        port,
        tls,
    })
}

fn run() -> Result<(), String> {
    let Some(a) = parse_args()? else { return Ok(()) };
    let b = parse_broker(&a.broker, a.ca_file.is_some() || a.cert.is_some())?;
    let client_id = a.client_id.unwrap_or_else(|| {
        let mut h = RandomState::new().build_hasher();
        h.write_u32(std::process::id());
        format!("fss-mqtt-{:08x}", h.finish() as u32)
    });

    let cfg = mqtt::Config {
        host: b.host,
        port: b.port,
        tls: b.tls || a.ca_file.is_some() || a.cert.is_some(),
        ca_file: a.ca_file,
        cert_file: a.cert,
        key_file: a.key,
        insecure: a.insecure,
        client_id,
        username: a.username,
        password: a.password,
        topics: a.topics.clone(),
        qos: a.qos,
    };
    let tls = mqtt::tls_config(&cfg)?;

    let store = Arc::new(Mutex::new(store::Store::new(a.history, a.max_payload)));
    mqtt::spawn(cfg, tls, store.clone());

    let opts = app::Options {
        broker: b.display,
        topics: a.topics,
        preview_bytes: a.preview,
        inline_bytes: a.inline,
    };
    let mut app = app::App::new(store, opts, theme::Theme::detect());

    let mut terminal = ratatui::init();
    let _ = execute!(stdout(), EnableBracketedPaste);
    let res = event_loop(&mut terminal, &mut app);
    let _ = execute!(stdout(), DisableBracketedPaste);
    ratatui::restore();
    res.map_err(|e| e.to_string())
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut app::App) -> std::io::Result<()> {
    let size = terminal.size()?;
    app.resize(size.width, size.height);
    app.tick();
    loop {
        terminal.draw(|f| ui::draw(f, app))?;
        if event::poll(FRAME)? {
            // Handle everything already queued, then redraw once.
            loop {
                match event::read()? {
                    Event::Key(k) if k.kind != KeyEventKind::Release => app.key(k),
                    Event::Paste(s) => app.paste(&s),
                    Event::Resize(w, h) => app.resize(w, h),
                    _ => {}
                }
                if app.quit {
                    return Ok(());
                }
                if !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }
        app.tick();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broker_urls() {
        let b = parse_broker("127.0.0.1:8883", false).unwrap();
        assert!(b.tls && b.port == 8883 && b.host == "127.0.0.1");
        let b = parse_broker("broker", true).unwrap();
        assert!(b.tls && b.port == 8883);
        let b = parse_broker("mqtt://u:p@host", false).unwrap();
        assert!(!b.tls && b.port == 1883 && b.host == "host" && b.display == "mqtt://host:1883");
        let b = parse_broker("mqtts://[::1]:9000", false).unwrap();
        assert!(b.tls && b.port == 9000 && b.host == "::1");
        assert!(parse_broker("wss://x", false).is_err());
        assert_eq!(parse_size("64KiB"), Ok(65536));
        assert_eq!(parse_size("2m"), Ok(2 << 20));
    }
}
