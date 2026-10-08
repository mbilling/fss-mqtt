//! A minimal MQTT v5 subscriber: connect (TCP or TLS), subscribe, receive,
//! keep alive, reconnect. Blocking I/O on one thread.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};

use crate::store::{ConnState, Props, Status, Store};

pub struct Config {
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub ca_file: Option<String>,
    pub cert_file: Option<String>,
    pub key_file: Option<String>,
    pub insecure: bool,
    pub client_id: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub topics: Vec<String>,
    pub qos: u8,
}

const KEEP_ALIVE: u16 = 30;
const RETRY: Duration = Duration::from_secs(2);

/// Builds the TLS config up front so bad cert paths fail at startup.
pub fn tls_config(cfg: &Config) -> Result<Option<Arc<rustls::ClientConfig>>, String> {
    if !cfg.tls {
        return Ok(None);
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?;

    let builder = if cfg.insecure {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerify(provider)))
    } else {
        let mut roots = rustls::RootCertStore::empty();
        match &cfg.ca_file {
            Some(path) => {
                let certs: Vec<_> = CertificateDer::pem_file_iter(path)
                    .map_err(|e| format!("--cafile: {e}"))?
                    .collect::<Result<_, _>>()
                    .map_err(|e| format!("--cafile: {e}"))?;
                if certs.is_empty() {
                    return Err(format!("--cafile: no PEM certificates found in {path}"));
                }
                for c in certs {
                    roots.add(c).map_err(|e| format!("--cafile: {e}"))?;
                }
            }
            None => {
                for c in rustls_native_certs::load_native_certs().certs {
                    let _ = roots.add(c);
                }
            }
        }
        builder.with_root_certificates(roots)
    };

    let config = match (&cfg.cert_file, &cfg.key_file) {
        (Some(cert), Some(key)) => {
            let chain: Vec<_> = CertificateDer::pem_file_iter(cert)
                .map_err(|e| format!("--cert: {e}"))?
                .collect::<Result<_, _>>()
                .map_err(|e| format!("--cert: {e}"))?;
            let key = PrivateKeyDer::from_pem_file(key).map_err(|e| format!("--key: {e}"))?;
            builder
                .with_client_auth_cert(chain, key)
                .map_err(|e| format!("client certificate: {e}"))?
        }
        (None, None) => builder.with_no_client_auth(),
        _ => return Err("--cert and --key must be given together".into()),
    };
    Ok(Some(Arc::new(config)))
}

#[derive(Debug)]
struct NoVerify(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        msg: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(msg, cert, dss, &self.0.signature_verification_algorithms)
    }
    fn verify_tls13_signature(
        &self,
        msg: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(msg, cert, dss, &self.0.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

enum Conn {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Conn {
    fn tcp(&self) -> &TcpStream {
        match self {
            Conn::Plain(s) => s,
            Conn::Tls(s) => &s.sock,
        }
    }
}

impl Read for Conn {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        match self {
            Conn::Plain(s) => s.read(b),
            Conn::Tls(s) => s.read(b),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        match self {
            Conn::Plain(s) => s.write(b),
            Conn::Tls(s) => s.write(b),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Conn::Plain(s) => s.flush(),
            Conn::Tls(s) => s.flush(),
        }
    }
}

/// Runs forever on its own thread, reconnecting after failures.
pub fn spawn(cfg: Config, tls: Option<Arc<rustls::ClientConfig>>, store: Arc<Mutex<Store>>) {
    std::thread::spawn(move || {
        loop {
            let err = session(&cfg, &tls, &store);
            let mut st = store.lock().unwrap();
            let was_up = st.status.state == ConnState::Connected;
            st.status = Status {
                state: if was_up {
                    ConnState::Disconnected
                } else {
                    ConnState::Connecting
                },
                err: Some(err),
            };
            drop(st);
            std::thread::sleep(RETRY);
        }
    });
}

fn set_status(store: &Mutex<Store>, state: ConnState, err: Option<String>) {
    store.lock().unwrap().status = Status { state, err };
}

/// One connection's lifetime. Returns why it ended.
fn session(cfg: &Config, tls: &Option<Arc<rustls::ClientConfig>>, store: &Mutex<Store>) -> String {
    let tcp = match TcpStream::connect((cfg.host.as_str(), cfg.port)) {
        Ok(s) => s,
        Err(e) => return format!("connect {}:{}: {e}", cfg.host, cfg.port),
    };
    let _ = tcp.set_nodelay(true);
    let mut conn = match tls {
        None => Conn::Plain(tcp),
        Some(tc) => {
            let name = match ServerName::try_from(cfg.host.clone()) {
                Ok(n) => n,
                Err(e) => return format!("tls: {e}"),
            };
            match rustls::ClientConnection::new(tc.clone(), name) {
                Ok(c) => Conn::Tls(Box::new(rustls::StreamOwned::new(c, tcp))),
                Err(e) => return format!("tls: {e}"),
            }
        }
    };
    let _ = conn.tcp().set_read_timeout(Some(Duration::from_secs(5)));

    if let Err(e) = conn.write_all(&connect_packet(cfg)).and_then(|_| conn.flush()) {
        return tidy_io(e);
    }

    let mut rd = Reader::default();
    let (kind, body) = match rd.next(&mut conn) {
        Ok(Some(p)) => p,
        Ok(None) => return "timed out waiting for CONNACK".into(),
        Err(e) => return tidy_io(e),
    };
    if kind >> 4 != 2 || body.len() < 2 {
        return "unexpected reply to CONNECT".into();
    }
    let mut keep_alive = KEEP_ALIVE;
    let mut reason_str = None;
    let mut b = Buf(&body[2..]);
    if let Some(props) = b.props() {
        for (id, v) in props {
            match (id, v) {
                (0x13, Prop::Int(k)) => keep_alive = k as u16,
                (0x1F, Prop::Str(s)) => reason_str = Some(s),
                _ => {}
            }
        }
    }
    if body[1] != 0 {
        let mut msg = format!("broker refused connection: {}", connack_reason(body[1]));
        if let Some(r) = reason_str {
            msg += &format!(": {r}");
        }
        return msg;
    }
    set_status(store, ConnState::Connected, None);

    // One SUBSCRIBE per filter, so a refused one is named and the rest work.
    let mut pending: Vec<(u16, String)> = Vec::new();
    for (i, t) in cfg.topics.iter().enumerate() {
        let pid = i as u16 + 1;
        if let Err(e) = conn.write_all(&subscribe_packet(pid, t, cfg.qos)) {
            return tidy_io(e);
        }
        pending.push((pid, t.clone()));
    }
    let _ = conn.flush();
    let mut denied: Vec<String> = Vec::new();

    let _ = conn.tcp().set_read_timeout(Some(Duration::from_millis(500)));
    let ping_every = Duration::from_secs((keep_alive.max(2) as u64) * 3 / 4);
    let mut last_send = Instant::now();
    let mut last_recv = Instant::now();

    loop {
        let pkt = match rd.next(&mut conn) {
            Ok(p) => p,
            Err(e) => return tidy_io(e),
        };
        if let Some((kind, body)) = pkt {
            last_recv = Instant::now();
            let mut reply: Option<[u8; 4]> = None;
            match kind >> 4 {
                3 => {
                    let qos = (kind >> 1) & 3;
                    let retain = kind & 1 == 1;
                    let Some((topic, pid, props, payload)) = parse_publish(&body, qos) else {
                        return "malformed PUBLISH".into();
                    };
                    store.lock().unwrap().add(&topic, payload, qos, retain, props);
                    match qos {
                        1 => reply = Some([0x40, 2, (pid >> 8) as u8, pid as u8]),
                        2 => reply = Some([0x50, 2, (pid >> 8) as u8, pid as u8]),
                        _ => {}
                    }
                }
                6 if body.len() >= 2 => reply = Some([0x70, 2, body[0], body[1]]), // PUBREL → PUBCOMP
                9 if body.len() >= 2 => {
                    // SUBACK
                    let pid = u16::from_be_bytes([body[0], body[1]]);
                    let mut b = Buf(&body[2..]);
                    let _ = b.props();
                    let code = b.0.first().copied().unwrap_or(0x80);
                    if let Some(i) = pending.iter().position(|(p, _)| *p == pid) {
                        let (_, t) = pending.remove(i);
                        if code >= 0x80 {
                            denied.push(format!("{t} ({})", suback_reason(code)));
                        }
                    }
                    if pending.is_empty() && !denied.is_empty() {
                        set_status(
                            store,
                            ConnState::Connected,
                            Some(format!("subscribe refused: {}", denied.join(", "))),
                        );
                    }
                }
                14 => {
                    let code = body.first().copied().unwrap_or(0);
                    return format!("broker disconnected: {}", connack_reason(code));
                }
                _ => {}
            }
            if let Some(r) = reply {
                if let Err(e) = conn.write_all(&r) {
                    return tidy_io(e);
                }
                last_send = Instant::now();
            }
            if rd.buffered() {
                continue; // drain what we already have before flushing
            }
            let _ = conn.flush();
        }
        if last_send.elapsed() >= ping_every {
            if let Err(e) = conn.write_all(&[0xC0, 0]).and_then(|_| conn.flush()) {
                return tidy_io(e);
            }
            last_send = Instant::now();
        }
        if last_recv.elapsed() > Duration::from_secs(keep_alive.max(2) as u64 * 2) {
            return "keepalive timeout".into();
        }
    }
}

fn tidy_io(e: io::Error) -> String {
    match e.kind() {
        io::ErrorKind::UnexpectedEof => "connection closed by broker".into(),
        _ => e.to_string(),
    }
}

/// Accumulates bytes and yields complete packets; a read timeout yields None.
#[derive(Default)]
struct Reader {
    buf: Vec<u8>,
    pos: usize,
}

impl Reader {
    fn buffered(&self) -> bool {
        self.pos < self.buf.len()
    }

    fn try_take(&mut self) -> Option<(u8, Vec<u8>)> {
        let b = &self.buf[self.pos..];
        if b.len() < 2 {
            return None;
        }
        let (mut len, mut mult, mut i) = (0usize, 1usize, 1);
        loop {
            let c = *b.get(i)?;
            len += (c & 0x7f) as usize * mult;
            i += 1;
            if c & 0x80 == 0 {
                break;
            }
            mult *= 128;
            if i > 4 {
                return None;
            }
        }
        if b.len() < i + len {
            return None;
        }
        let pkt = (b[0], b[i..i + len].to_vec());
        self.pos += i + len;
        if self.pos == self.buf.len() {
            self.buf.clear();
            self.pos = 0;
            // A large packet grew the buffer; give that memory back.
            if self.buf.capacity() > 1 << 20 {
                self.buf = Vec::new();
            }
        }
        Some(pkt)
    }

    fn next(&mut self, r: &mut impl Read) -> io::Result<Option<(u8, Vec<u8>)>> {
        loop {
            if let Some(p) = self.try_take() {
                return Ok(Some(p));
            }
            if self.pos > 0 {
                self.buf.drain(..self.pos);
                self.pos = 0;
            }
            let mut tmp = [0u8; 16 * 1024];
            match r.read(&mut tmp) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => self.buf.extend_from_slice(&tmp[..n]),
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => return Ok(None),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }
}

// ── Encoding ───────────────────────────────────────────────────────────────

fn put_varint(out: &mut Vec<u8>, mut n: usize) {
    loop {
        let mut b = (n % 128) as u8;
        n /= 128;
        if n > 0 {
            b |= 0x80;
        }
        out.push(b);
        if n == 0 {
            break;
        }
    }
}

fn put_str(out: &mut Vec<u8>, s: &[u8]) {
    out.extend_from_slice(&(s.len() as u16).to_be_bytes());
    out.extend_from_slice(s);
}

fn packet(header: u8, body: Vec<u8>) -> Vec<u8> {
    let mut out = vec![header];
    put_varint(&mut out, body.len());
    out.extend(body);
    out
}

fn connect_packet(cfg: &Config) -> Vec<u8> {
    let mut b = Vec::new();
    put_str(&mut b, b"MQTT");
    b.push(5);
    let mut flags = 0x02; // clean start
    if cfg.username.is_some() {
        flags |= 0x80;
    }
    if cfg.password.is_some() {
        flags |= 0x40;
    }
    b.push(flags);
    b.extend_from_slice(&KEEP_ALIVE.to_be_bytes());
    b.push(0); // no properties
    put_str(&mut b, cfg.client_id.as_bytes());
    if let Some(u) = &cfg.username {
        put_str(&mut b, u.as_bytes());
    }
    if let Some(p) = &cfg.password {
        put_str(&mut b, p.as_bytes());
    }
    packet(0x10, b)
}

fn subscribe_packet(pid: u16, topic: &str, qos: u8) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&pid.to_be_bytes());
    b.push(0); // no properties
    put_str(&mut b, topic.as_bytes());
    b.push(qos);
    packet(0x82, b)
}

// ── Decoding ───────────────────────────────────────────────────────────────

struct Buf<'a>(&'a [u8]);

enum Prop {
    Int(u32),
    Str(String),
    Bin(Vec<u8>),
    Pair(String, String),
}

impl<'a> Buf<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Some(a)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        let b = self.take(2)?;
        Some(u16::from_be_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        let b = self.take(4)?;
        Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn varint(&mut self) -> Option<usize> {
        let (mut n, mut mult) = (0usize, 1usize);
        for _ in 0..4 {
            let c = self.u8()?;
            n += (c & 0x7f) as usize * mult;
            if c & 0x80 == 0 {
                return Some(n);
            }
            mult *= 128;
        }
        None
    }
    fn bin(&mut self) -> Option<&'a [u8]> {
        let n = self.u16()? as usize;
        self.take(n)
    }
    fn str(&mut self) -> Option<String> {
        Some(String::from_utf8_lossy(self.bin()?).into_owned())
    }

    fn props(&mut self) -> Option<Vec<(u8, Prop)>> {
        let n = self.varint()?;
        let mut b = Buf(self.take(n)?);
        let mut out = Vec::new();
        while !b.0.is_empty() {
            let id = b.u8()?;
            let v = match id {
                0x01 | 0x17 | 0x19 | 0x24 | 0x25 | 0x28 | 0x29 | 0x2A => Prop::Int(b.u8()? as u32),
                0x13 | 0x21 | 0x22 | 0x23 => Prop::Int(b.u16()? as u32),
                0x02 | 0x11 | 0x18 | 0x27 => Prop::Int(b.u32()?),
                0x0B => Prop::Int(b.varint()? as u32),
                0x03 | 0x08 | 0x12 | 0x15 | 0x1A | 0x1C | 0x1F => Prop::Str(b.str()?),
                0x09 | 0x16 => Prop::Bin(b.bin()?.to_vec()),
                0x26 => Prop::Pair(b.str()?, b.str()?),
                _ => return None,
            };
            out.push((id, v));
        }
        Some(out)
    }
}

fn parse_publish(body: &[u8], qos: u8) -> Option<(String, u16, Props, &[u8])> {
    let mut b = Buf(body);
    let topic = b.str()?;
    let pid = if qos > 0 { b.u16()? } else { 0 };
    let mut p = Props::default();
    for (id, v) in b.props()? {
        match (id, v) {
            (0x01, Prop::Int(v)) => p.payload_format = Some(v as u8),
            (0x02, Prop::Int(v)) => p.message_expiry = Some(v),
            (0x03, Prop::Str(v)) => p.content_type = Some(v),
            (0x08, Prop::Str(v)) => p.response_topic = Some(v),
            (0x09, Prop::Bin(v)) => p.correlation_data = Some(v),
            (0x0B, Prop::Int(v)) => p.subscription_id = Some(v),
            (0x26, Prop::Pair(k, v)) => p.user.push((k, v)),
            _ => {}
        }
    }
    Some((topic, pid, p, b.0))
}

fn connack_reason(code: u8) -> String {
    match code {
        0x80 => "unspecified error",
        0x81 => "malformed packet",
        0x82 => "protocol error",
        0x84 => "unsupported protocol version",
        0x85 => "client id not valid",
        0x86 => "bad username or password",
        0x87 => "not authorized",
        0x88 => "server unavailable",
        0x89 => "server busy",
        0x8A => "banned",
        0x8B => "server shutting down",
        0x8C => "bad authentication method",
        0x8D => "keepalive timeout",
        0x8E => "session taken over",
        0x95 => "packet too large",
        0x97 => "quota exceeded",
        0x9F => "connection rate exceeded",
        _ => return format!("reason code {code}"),
    }
    .into()
}

fn suback_reason(code: u8) -> String {
    match code {
        0x80 => "unspecified error",
        0x83 => "implementation specific error",
        0x87 => "not authorized",
        0x8F => "topic filter invalid",
        0x91 => "packet id in use",
        0x97 => "quota exceeded",
        0x9E => "shared subscriptions not supported",
        0xA1 => "subscription ids not supported",
        0xA2 => "wildcards not supported",
        _ => return format!("reason code {code}"),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_roundtrip() {
        // topic "a/b", qos1 pid 7, props: content-type "t", user ("k","v"), payload "hi"
        let mut body = Vec::new();
        put_str(&mut body, b"a/b");
        body.extend_from_slice(&7u16.to_be_bytes());
        let mut props = vec![0x03];
        put_str(&mut props, b"t");
        props.push(0x26);
        put_str(&mut props, b"k");
        put_str(&mut props, b"v");
        put_varint(&mut body, props.len());
        body.extend(props);
        body.extend_from_slice(b"hi");

        let (topic, pid, p, payload) = parse_publish(&body, 1).unwrap();
        assert_eq!((topic.as_str(), pid, payload), ("a/b", 7, &b"hi"[..]));
        assert_eq!(p.content_type.as_deref(), Some("t"));
        assert_eq!(p.user, vec![("k".to_string(), "v".to_string())]);
    }

    #[test]
    fn reader_releases_large_buffer() {
        let data = packet(0x30, vec![0u8; 3 << 20]);
        let mut rd = Reader::default();
        let mut src = &data[..];
        assert_eq!(rd.next(&mut src).unwrap().unwrap().1.len(), 3 << 20);
        assert!(rd.buf.capacity() <= 1 << 20, "buffer kept {} bytes", rd.buf.capacity());
    }

    #[test]
    fn reader_splits_packets() {
        let mut data = packet(0x30, vec![0, 1, b'x', 0, b'p']);
        data.extend(packet(0xD0, vec![]));
        let mut rd = Reader::default();
        let mut src = &data[..];
        assert_eq!(rd.next(&mut src).unwrap().unwrap().0, 0x30);
        assert_eq!(rd.next(&mut src).unwrap().unwrap().0, 0xD0);
    }
}

/// Needs mosquitto on 127.0.0.1:18830 (plain) and localhost:18883 (TLS, with
/// the CA in $FSS_TEST_CA). CI runs it; locally:
/// FSS_TEST_CA=ca.pem cargo test --release e2e -- --ignored --nocapture
#[cfg(test)]
mod e2e {
    use super::*;
    use crate::store::Message;
    use std::process::Command;

    fn cfg(port: u16, tls: bool, ca: Option<&str>, id: &str) -> Config {
        Config {
            host: if tls { "localhost".into() } else { "127.0.0.1".into() },
            port,
            tls,
            ca_file: ca.map(String::from),
            cert_file: None,
            key_file: None,
            insecure: false,
            client_id: id.into(),
            username: None,
            password: None,
            topics: vec!["#".into()],
            qos: 2,
        }
    }

    /// Waits until connected and subscribed, or until an error is reported.
    fn wait_status(st: &Arc<Mutex<Store>>) -> Status {
        for _ in 0..50 {
            let s = st.lock().unwrap().status.clone();
            if s.state == ConnState::Connected || s.err.is_some() {
                std::thread::sleep(Duration::from_millis(300)); // let SUBACK land
                return st.lock().unwrap().status.clone();
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        st.lock().unwrap().status.clone()
    }

    fn wait_for(st: &Arc<Mutex<Store>>, total: u64) -> u64 {
        let start = std::time::Instant::now();
        loop {
            let n = st.lock().unwrap().total();
            if n >= total || start.elapsed() > Duration::from_secs(10) {
                return n;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn publish(args: &[&str]) {
        let ok = Command::new("mosquitto_pub")
            .args(args)
            .status()
            .expect("mosquitto_pub")
            .success();
        assert!(ok, "mosquitto_pub {args:?} failed");
    }

    fn latest(st: &Arc<Mutex<Store>>, path: &str) -> Option<Arc<Message>> {
        let st = st.lock().unwrap();
        st.nodes
            .iter()
            .find(|n| n.path == path)
            .and_then(|n| n.latest().cloned())
    }

    #[test]
    #[ignore]
    fn e2e() {
        // Plain TCP: QoS 1 and 2 with v5 properties.
        let st = Arc::new(Mutex::new(Store::new(10, 64 * 1024)));
        spawn(cfg(18830, false, None, "e2e-plain"), None, st.clone());
        let s = wait_status(&st);
        assert!(
            s.state == ConnState::Connected && s.err.is_none(),
            "plain connect: {:?}",
            s.err
        );

        for q in ["1", "2"] {
            let topic = format!("e2e/plant/qos{q}");
            publish(&[
                "-p",
                "18830",
                "-V",
                "mqttv5",
                "-q",
                q,
                "-t",
                &topic,
                "-m",
                r#"{"t":21.5}"#,
                "-D",
                "publish",
                "user-property",
                "site",
                "oslo",
                "-D",
                "publish",
                "content-type",
                "application/json",
            ]);
        }
        assert_eq!(wait_for(&st, 2), 2);
        for q in [1u8, 2] {
            let m = latest(&st, &format!("e2e/plant/qos{q}")).expect("message stored");
            assert_eq!(m.qos, q);
            assert_eq!(m.payload, br#"{"t":21.5}"#);
            assert_eq!(m.props.user, vec![("site".to_string(), "oslo".to_string())]);
            assert_eq!(m.props.content_type.as_deref(), Some("application/json"));
        }

        // A QoS 1 burst must arrive complete (QoS 0 bursts may be dropped by the broker).
        let n = 5_000;
        let ok = Command::new("sh")
            .arg("-c")
            .arg(format!(
                r#"seq 1 {n} | sed 's/.*/{{"v":&}}/' | mosquitto_pub -p 18830 -q 1 -t e2e/load -l"#
            ))
            .status()
            .unwrap()
            .success();
        assert!(ok);
        assert_eq!(wait_for(&st, n + 2), n + 2, "burst incomplete");

        // TLS with the test CA connects and receives; without it the cert is refused.
        let ca = std::env::var("FSS_TEST_CA").expect("set FSS_TEST_CA");
        let st = Arc::new(Mutex::new(Store::new(10, 1024)));
        let c = cfg(18883, true, Some(&ca), "e2e-tls");
        let tls = tls_config(&c).unwrap();
        spawn(c, tls, st.clone());
        let s = wait_status(&st);
        assert!(
            s.state == ConnState::Connected && s.err.is_none(),
            "tls connect: {:?}",
            s.err
        );
        publish(&[
            "-p",
            "18883",
            "-h",
            "localhost",
            "--cafile",
            &ca,
            "-t",
            "e2e/tls",
            "-m",
            "hi",
        ]);
        assert_eq!(wait_for(&st, 1), 1);

        let st = Arc::new(Mutex::new(Store::new(10, 1024)));
        let c = cfg(18883, true, None, "e2e-tls-noca");
        let tls = tls_config(&c).unwrap();
        spawn(c, tls, st.clone());
        let s = wait_status(&st);
        assert!(s.state != ConnState::Connected, "connected without trusting the CA");
        let err = s.err.unwrap_or_default();
        assert!(
            err.contains("certificate") || err.contains("UnknownIssuer"),
            "unexpected error: {err}"
        );
    }
}
