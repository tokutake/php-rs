//! A MySQL / MariaDB client speaking the wire protocol over TCP or a Unix
//! socket: the handshake with mysql_native_password authentication, text
//! queries with their result sets (several per query too), and the other
//! commands mysqli needs

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::time::Duration;

const CLIENT_LONG_PASSWORD: u32 = 1;
const CLIENT_FOUND_ROWS: u32 = 2;
const CLIENT_LONG_FLAG: u32 = 4;
const CLIENT_CONNECT_WITH_DB: u32 = 8;
const CLIENT_PROTOCOL_41: u32 = 512;
const CLIENT_TRANSACTIONS: u32 = 8192;
const CLIENT_SECURE_CONNECTION: u32 = 32768;
const CLIENT_MULTI_STATEMENTS: u32 = 1 << 16;
const CLIENT_MULTI_RESULTS: u32 = 1 << 17;
const CLIENT_PLUGIN_AUTH: u32 = 1 << 19;
const CLIENT_PLUGIN_AUTH_LENENC_CLIENT_DATA: u32 = 1 << 21;

const SERVER_MORE_RESULTS_EXISTS: u16 = 8;
const SERVER_STATUS_NO_BACKSLASH_ESCAPES: u16 = 512;

/// utf8mb4_general_ci
pub const UTF8MB4: u8 = 45;

/// A server error, or one of the client's own (2002 and friends)
#[derive(Debug, Clone)]
pub struct Error {
    pub code: u16,
    pub sqlstate: String,
    pub message: String,
}

impl Error {
    fn client(code: u16, message: impl Into<String>) -> Error {
        Error { code, sqlstate: "HY000".into(), message: message.into() }
    }
}

fn lost() -> Error {
    Error::client(2006, "MySQL server has gone away")
}

enum Stream {
    Tcp(TcpStream),
    Unix(UnixStream),
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Stream::Tcp(s) => s.read(buf),
            Stream::Unix(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Stream::Tcp(s) => s.write(buf),
            Stream::Unix(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Stream::Tcp(s) => s.flush(),
            Stream::Unix(s) => s.flush(),
        }
    }
}

/// A column of a result set, as its definition packet describes it
#[derive(Debug, Clone, Default)]
pub struct Column {
    pub catalog: Vec<u8>,
    pub db: Vec<u8>,
    pub table: Vec<u8>,
    pub org_table: Vec<u8>,
    pub name: Vec<u8>,
    pub org_name: Vec<u8>,
    pub charset: u16,
    pub length: u32,
    pub ty: u8,
    pub flags: u16,
    pub decimals: u8,
}

/// The outcome of one statement: rows, or what an OK packet reports
#[derive(Debug, Clone, Default)]
pub struct QueryResult {
    pub columns: Option<Vec<Column>>,
    pub rows: Vec<Vec<Option<Vec<u8>>>>,
    pub affected_rows: u64,
    pub insert_id: u64,
    pub warnings: u16,
    pub info: Vec<u8>,
}

pub struct Connection {
    stream: Stream,
    seq: u8,
    pub server_version: String,
    pub thread_id: u32,
    pub status: u16,
    pub charset: u8,
    /// Results of a multi-statement query not fetched yet
    pending: bool,
}

/// Bytes read from a packet payload
struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn u8(&mut self) -> u8 {
        let b = self.buf.get(self.pos).copied().unwrap_or(0);
        self.pos += 1;
        b
    }

    fn u16(&mut self) -> u16 {
        u16::from_le_bytes([self.u8(), self.u8()])
    }

    fn u32(&mut self) -> u32 {
        u32::from_le_bytes([self.u8(), self.u8(), self.u8(), self.u8()])
    }

    fn bytes(&mut self, n: usize) -> &'a [u8] {
        let start = self.pos.min(self.buf.len());
        let end = (self.pos + n).min(self.buf.len());
        self.pos += n;
        &self.buf[start..end]
    }

    fn nul_str(&mut self) -> &'a [u8] {
        let rest = &self.buf[self.pos.min(self.buf.len())..];
        let n = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        self.pos += n + 1;
        &rest[..n]
    }

    fn rest(&mut self) -> &'a [u8] {
        let r = &self.buf[self.pos.min(self.buf.len())..];
        self.pos = self.buf.len();
        r
    }

    /// A length-encoded integer; None for the NULL marker 0xfb
    fn lenenc(&mut self) -> Option<u64> {
        match self.u8() {
            0xfb => None,
            0xfc => Some(self.u16() as u64),
            0xfd => Some(u32::from_le_bytes([self.u8(), self.u8(), self.u8(), 0]) as u64),
            0xfe => Some(u64::from_le_bytes(std::array::from_fn(|_| self.u8()))),
            n => Some(n as u64),
        }
    }

    fn lenenc_str(&mut self) -> Option<&'a [u8]> {
        let n = self.lenenc()?;
        Some(self.bytes(n as usize))
    }
}

fn lenenc(out: &mut Vec<u8>, n: u64) {
    match n {
        0..=250 => out.push(n as u8),
        251..=0xffff => {
            out.push(0xfc);
            out.extend_from_slice(&(n as u16).to_le_bytes());
        }
        0x10000..=0xff_ffff => {
            out.push(0xfd);
            out.extend_from_slice(&(n as u32).to_le_bytes()[..3]);
        }
        _ => {
            out.push(0xfe);
            out.extend_from_slice(&n.to_le_bytes());
        }
    }
}

/// mysql_native_password: SHA1(password) XOR SHA1(scramble + SHA1(SHA1(password)))
fn native_password(password: &[u8], scramble: &[u8]) -> Vec<u8> {
    if password.is_empty() {
        return Vec::new();
    }
    let sha1 = crate::builtins::sha1;
    let stage1 = sha1(password);
    let stage2 = sha1(&stage1);
    let mut salted = scramble.to_vec();
    salted.extend_from_slice(&stage2);
    let mask = sha1(&salted);
    stage1.iter().zip(mask).map(|(a, b)| a ^ b).collect()
}

fn parse_err(payload: &[u8]) -> Error {
    let mut c = Cursor { buf: payload, pos: 1 };
    let code = c.u16();
    let sqlstate = if c.buf.get(c.pos) == Some(&b'#') {
        c.pos += 1;
        String::from_utf8_lossy(c.bytes(5)).into_owned()
    } else {
        "HY000".into()
    };
    Error { code, sqlstate, message: String::from_utf8_lossy(c.rest()).into_owned() }
}

impl Connection {
    /// Connects and logs in. An empty or "localhost" host goes through the Unix socket
    pub fn connect(host: &str, port: u16, socket: &str, user: &str, password: &[u8], db: &str) -> Result<Connection, Error> {
        let stream = if host.is_empty() || host == "localhost" {
            let path = if socket.is_empty() { "/tmp/mysql.sock" } else { socket };
            UnixStream::connect(path).map(Stream::Unix).map_err(|e| {
                Error::client(2002, crate::builtins::strerror(&e))
            })?
        } else {
            let host = host.trim_start_matches("p:");
            let addr = if host.contains(':') && !host.starts_with('[') { format!("[{host}]:{port}") } else { format!("{host}:{port}") };
            let s = TcpStream::connect(&addr).map_err(|e| Error::client(2002, crate::builtins::strerror(&e)))?;
            let _ = s.set_nodelay(true);
            Stream::Tcp(s)
        };
        let mut conn =
            Connection { stream, seq: 0, server_version: String::new(), thread_id: 0, status: 0, charset: UTF8MB4, pending: false };
        let _ = conn.set_timeout(Some(Duration::from_secs(60)));
        conn.handshake(user, password, db)?;
        Ok(conn)
    }

    fn set_timeout(&self, d: Option<Duration>) -> io::Result<()> {
        match &self.stream {
            Stream::Tcp(s) => {
                s.set_read_timeout(d)?;
                s.set_write_timeout(d)
            }
            Stream::Unix(s) => {
                s.set_read_timeout(d)?;
                s.set_write_timeout(d)
            }
        }
    }

    fn read_packet(&mut self) -> Result<Vec<u8>, Error> {
        let mut payload = Vec::new();
        loop {
            let mut header = [0u8; 4];
            self.stream.read_exact(&mut header).map_err(|_| lost())?;
            let len = u32::from_le_bytes([header[0], header[1], header[2], 0]) as usize;
            self.seq = header[3].wrapping_add(1);
            let start = payload.len();
            payload.resize(start + len, 0);
            self.stream.read_exact(&mut payload[start..]).map_err(|_| lost())?;
            // A payload of 2^24-1 bytes continues in the next packet
            if len < 0xff_ffff {
                return Ok(payload);
            }
        }
    }

    fn write_packet(&mut self, payload: &[u8]) -> Result<(), Error> {
        let mut out = Vec::with_capacity(payload.len() + 4);
        let mut chunks = payload.chunks(0xff_ffff).peekable();
        if payload.is_empty() {
            out.extend_from_slice(&[0, 0, 0, self.seq]);
            self.seq = self.seq.wrapping_add(1);
        }
        while let Some(chunk) = chunks.next() {
            out.extend_from_slice(&(chunk.len() as u32).to_le_bytes()[..3]);
            out.push(self.seq);
            self.seq = self.seq.wrapping_add(1);
            out.extend_from_slice(chunk);
            // A full last chunk is followed by an empty one
            if chunks.peek().is_none() && chunk.len() == 0xff_ffff {
                out.extend_from_slice(&[0, 0, 0, self.seq]);
                self.seq = self.seq.wrapping_add(1);
            }
        }
        self.stream.write_all(&out).map_err(|_| lost())?;
        self.stream.flush().map_err(|_| lost())
    }

    fn handshake(&mut self, user: &str, password: &[u8], db: &str) -> Result<(), Error> {
        let greeting = self.read_packet()?;
        if greeting.first() == Some(&0xff) {
            return Err(parse_err(&greeting));
        }
        let mut c = Cursor { buf: &greeting, pos: 0 };
        let protocol = c.u8();
        if protocol != 10 {
            return Err(Error::client(2012, "Error in server handshake"));
        }
        self.server_version = String::from_utf8_lossy(c.nul_str()).into_owned();
        self.thread_id = c.u32();
        let mut scramble = c.bytes(8).to_vec();
        c.u8();
        let caps_low = c.u16() as u32;
        let _charset = c.u8();
        self.status = c.u16();
        let caps = caps_low | (c.u16() as u32) << 16;
        let auth_len = c.u8() as usize;
        c.bytes(10);
        if caps & CLIENT_SECURE_CONNECTION != 0 {
            let n = auth_len.saturating_sub(8).max(13);
            let part2 = c.bytes(n);
            // The second part ends with a NUL that is not part of the scramble
            scramble.extend_from_slice(part2.strip_suffix(&[0]).unwrap_or(part2));
        }
        let plugin = if caps & CLIENT_PLUGIN_AUTH != 0 { c.nul_str().to_vec() } else { b"mysql_native_password".to_vec() };

        let mut flags = CLIENT_LONG_PASSWORD
            | CLIENT_FOUND_ROWS
            | CLIENT_LONG_FLAG
            | CLIENT_PROTOCOL_41
            | CLIENT_TRANSACTIONS
            | CLIENT_SECURE_CONNECTION
            | CLIENT_MULTI_STATEMENTS
            | CLIENT_MULTI_RESULTS
            | CLIENT_PLUGIN_AUTH
            | CLIENT_PLUGIN_AUTH_LENENC_CLIENT_DATA;
        if !db.is_empty() {
            flags |= CLIENT_CONNECT_WITH_DB;
        }
        flags &= caps | CLIENT_CONNECT_WITH_DB;
        let auth = if plugin == b"mysql_native_password" { native_password(password, &scramble) } else { Vec::new() };
        let mut resp = Vec::new();
        resp.extend_from_slice(&flags.to_le_bytes());
        resp.extend_from_slice(&(16u32 << 20).to_le_bytes());
        resp.push(self.charset);
        resp.extend_from_slice(&[0; 23]);
        resp.extend_from_slice(user.as_bytes());
        resp.push(0);
        if flags & CLIENT_PLUGIN_AUTH_LENENC_CLIENT_DATA != 0 {
            lenenc(&mut resp, auth.len() as u64);
        } else {
            resp.push(auth.len() as u8);
        }
        resp.extend_from_slice(&auth);
        if flags & CLIENT_CONNECT_WITH_DB != 0 {
            resp.extend_from_slice(db.as_bytes());
            resp.push(0);
        }
        if flags & CLIENT_PLUGIN_AUTH != 0 {
            resp.extend_from_slice(&plugin);
            resp.push(0);
        }
        self.write_packet(&resp)?;
        loop {
            let reply = self.read_packet()?;
            match reply.first() {
                Some(0x00) => {
                    self.parse_ok(&reply);
                    return Ok(());
                }
                Some(0xff) => return Err(parse_err(&reply)),
                // An auth switch request: the server wants another method or scramble
                Some(0xfe) => {
                    let mut c = Cursor { buf: &reply, pos: 1 };
                    let plugin = c.nul_str().to_vec();
                    let data = c.rest();
                    let scramble = data.strip_suffix(&[0]).unwrap_or(data);
                    let auth = match plugin.as_slice() {
                        b"mysql_native_password" => native_password(password, scramble),
                        b"mysql_clear_password" => {
                            let mut p = password.to_vec();
                            p.push(0);
                            p
                        }
                        other => {
                            return Err(Error::client(
                                2059,
                                format!("Authentication plugin '{}' cannot be loaded", String::from_utf8_lossy(other)),
                            ));
                        }
                    };
                    self.write_packet(&auth)?;
                }
                _ => return Err(Error::client(2012, "Error in server handshake")),
            }
        }
    }

    fn parse_ok(&mut self, payload: &[u8]) -> QueryResult {
        let mut c = Cursor { buf: payload, pos: 1 };
        let affected_rows = c.lenenc().unwrap_or(0);
        let insert_id = c.lenenc().unwrap_or(0);
        self.status = c.u16();
        let warnings = c.u16();
        let info = c.rest().to_vec();
        QueryResult { columns: None, rows: Vec::new(), affected_rows, insert_id, warnings, info }
    }

    /// Sends a command; the sequence starts over
    fn command(&mut self, cmd: u8, arg: &[u8]) -> Result<(), Error> {
        // Results left from an earlier multi-statement query come first
        while self.pending {
            self.next_result()?;
        }
        self.seq = 0;
        let mut payload = Vec::with_capacity(arg.len() + 1);
        payload.push(cmd);
        payload.extend_from_slice(arg);
        self.write_packet(&payload)
    }

    /// Runs a query; its first result (more may follow, see more_results)
    pub fn query(&mut self, sql: &[u8]) -> Result<QueryResult, Error> {
        self.command(0x03, sql)?;
        self.read_result()
    }

    pub fn more_results(&self) -> bool {
        self.pending
    }

    /// The next result of a multi-statement query
    pub fn next_result(&mut self) -> Result<QueryResult, Error> {
        self.pending = false;
        self.read_result()
    }

    fn read_result(&mut self) -> Result<QueryResult, Error> {
        let first = self.read_packet()?;
        let result = match first.first() {
            Some(0x00) => self.parse_ok(&first),
            Some(0xff) => {
                self.pending = false;
                return Err(parse_err(&first));
            }
            Some(0xfb) => return Err(Error::client(2000, "LOAD DATA LOCAL INFILE is not supported")),
            _ => {
                let mut c = Cursor { buf: &first, pos: 0 };
                let n = c.lenenc().unwrap_or(0) as usize;
                let mut columns = Vec::with_capacity(n);
                for _ in 0..n {
                    let p = self.read_packet()?;
                    columns.push(parse_column(&p));
                }
                // The EOF packet after the column definitions
                let eof = self.read_packet()?;
                if eof.first() != Some(&0xfe) {
                    return Err(Error::client(2027, "Malformed packet"));
                }
                let mut rows = Vec::new();
                loop {
                    let p = self.read_packet()?;
                    match p.first() {
                        Some(0xfe) if p.len() < 9 => {
                            let mut c = Cursor { buf: &p, pos: 1 };
                            let warnings = c.u16();
                            self.status = c.u16();
                            self.pending = self.status & SERVER_MORE_RESULTS_EXISTS != 0;
                            return Ok(QueryResult { columns: Some(columns), rows, warnings, ..Default::default() });
                        }
                        Some(0xff) => return Err(parse_err(&p)),
                        _ => {
                            let mut c = Cursor { buf: &p, pos: 0 };
                            rows.push((0..n).map(|_| c.lenenc_str().map(<[u8]>::to_vec)).collect());
                        }
                    }
                }
            }
        };
        self.pending = self.status & SERVER_MORE_RESULTS_EXISTS != 0;
        Ok(result)
    }

    pub fn select_db(&mut self, db: &str) -> Result<(), Error> {
        self.command(0x02, db.as_bytes())?;
        let p = self.read_packet()?;
        match p.first() {
            Some(0xff) => Err(parse_err(&p)),
            _ => {
                self.parse_ok(&p);
                Ok(())
            }
        }
    }

    pub fn ping(&mut self) -> Result<(), Error> {
        self.command(0x0e, &[])?;
        let p = self.read_packet()?;
        if p.first() == Some(&0xff) { Err(parse_err(&p)) } else { Ok(()) }
    }

    pub fn close(&mut self) {
        let _ = self.command(0x01, &[]);
    }

    pub fn no_backslash_escapes(&self) -> bool {
        self.status & SERVER_STATUS_NO_BACKSLASH_ESCAPES != 0
    }

    /// The server version as mysqli_get_server_version() gives it: 10.11.6 -> 101106
    pub fn server_version_number(&self) -> i64 {
        let v = self.server_version.trim_start_matches("5.5.5-");
        let mut parts = v.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse::<i64>().unwrap_or(0));
        let (a, b, c) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0));
        a * 10000 + b * 100 + c
    }
}

fn parse_column(p: &[u8]) -> Column {
    let mut c = Cursor { buf: p, pos: 0 };
    let mut s = || c.lenenc_str().map(<[u8]>::to_vec).unwrap_or_default();
    let (catalog, db, table, org_table, name, org_name) = (s(), s(), s(), s(), s(), s());
    c.lenenc();
    Column { catalog, db, table, org_table, name, org_name, charset: c.u16(), length: c.u32(), ty: c.u8(), flags: c.u16(), decimals: c.u8() }
}

/// mysqli_real_escape_string
pub fn escape(s: &[u8], no_backslash: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() + 8);
    for &b in s {
        if no_backslash {
            if b == b'\'' {
                out.push(b'\'');
            }
            out.push(b);
            continue;
        }
        match b {
            0 => out.extend_from_slice(b"\\0"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\'' => out.extend_from_slice(b"\\'"),
            b'"' => out.extend_from_slice(b"\\\""),
            0x1a => out.extend_from_slice(b"\\Z"),
            b => out.push(b),
        }
    }
    out
}
