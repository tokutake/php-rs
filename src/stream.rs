//! Resources and streams: files, php://memory and php://temp, the standard
//! streams and directory handles.
//!
//! A resource holds its stream until it is closed (fclose, or the last
//! reference going away, which drops the file handle). Reads follow php-src's
//! end-of-file rule: a read that gets fewer bytes than it asked for sets EOF.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::rc::Rc;

/// How much plain file reading asks for at a time, as php-src reports it
pub const CHUNK: usize = 8192;
/// php-src's default memory_limit: no read may produce more than this
pub const READ_LIMIT: usize = 128 << 20;

pub struct Resource {
    pub id: i64,
    /// None once closed
    pub stream: RefCell<Option<Stream>>,
}

impl fmt::Debug for Resource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Resource({})", self.id)
    }
}

impl Resource {
    pub fn is_open(&self) -> bool {
        self.stream.try_borrow().map(|s| s.is_some()).unwrap_or(true)
    }

    /// The type get_resource_type() and var_dump() show
    pub fn type_name(&self) -> &'static str {
        if self.is_open() { "stream" } else { "Unknown" }
    }
}

thread_local! {
    /// Resource ids are never reused; 1-3 are STDIN, STDOUT and STDERR
    static NEXT_ID: Cell<i64> = const { Cell::new(4) };
    static CONTEXT: Cell<bool> = const { Cell::new(false) };
}

pub fn alloc_id() -> i64 {
    NEXT_ID.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    })
}

/// php-src creates the default stream context (taking a resource id) the
/// first time a function that accepts a context opens something
pub fn ensure_context() {
    if !CONTEXT.with(|c| c.replace(true)) {
        alloc_id();
    }
}

pub fn new_resource(stream: Stream) -> Rc<Resource> {
    Rc::new(Resource { id: alloc_id(), stream: RefCell::new(Some(stream)) })
}

pub enum Backend {
    File(File),
    Memory { data: Vec<u8>, pos: usize },
    Stdin,
    Stdout,
    Stderr,
    /// php://output: the same place echo writes to
    Output,
    Dir { entries: Vec<Vec<u8>>, pos: usize },
}

/// Where a stream writes standard output to: the interpreter's output
pub trait Sink {
    fn out(&mut self, bytes: &[u8]);
    fn flush_out(&mut self);
}

pub struct Stream {
    pub backend: Backend,
    pub mode: String,
    pub uri: Vec<u8>,
    pub wrapper_type: &'static str,
    pub stream_type: &'static str,
    pub readable: bool,
    pub writable: bool,
    pub append: bool,
    pub eof: bool,
    /// Bytes read or written, the position of a stream that cannot seek
    count: u64,
    /// Read-ahead for line reading from files and stdin
    rbuf: Vec<u8>,
    rpos: usize,
}

/// The result of a failed read or write: the errno and its message, for the
/// notice php-src gives
pub struct IoFail {
    pub errno: i32,
    pub msg: String,
}

impl From<io::Error> for IoFail {
    fn from(e: io::Error) -> Self {
        let errno = e.raw_os_error().unwrap_or(5);
        let msg = match errno {
            9 => "Bad file descriptor".to_string(),
            21 => "Is a directory".to_string(),
            _ => {
                let s = e.to_string();
                s.split(" (os error").next().unwrap_or(&s).to_string()
            }
        };
        IoFail { errno, msg }
    }
}

/// The read would go past READ_LIMIT
pub const OUT_OF_MEMORY: i32 = -1;

fn too_big() -> IoFail {
    IoFail { errno: OUT_OF_MEMORY, msg: String::new() }
}

fn bad_fd() -> IoFail {
    IoFail { errno: 9, msg: "Bad file descriptor".into() }
}

impl Stream {
    pub fn new(backend: Backend, mode: &str, uri: &[u8]) -> Stream {
        let (readable, writable, append) = mode_flags(mode).unwrap_or((true, false, false));
        let (wrapper_type, stream_type) = match backend {
            Backend::File(_) => ("plainfile", "STDIO"),
            Backend::Memory { .. } => ("PHP", if uri.starts_with(b"php://temp") { "TEMP" } else { "MEMORY" }),
            Backend::Stdin | Backend::Stdout | Backend::Stderr | Backend::Output => ("PHP", "STDIO"),
            Backend::Dir { .. } => ("plainfile", "dir"),
        };
        Stream {
            backend,
            mode: mode.to_string(),
            uri: uri.to_vec(),
            wrapper_type,
            stream_type,
            readable,
            writable,
            append,
            eof: false,
            count: 0,
            rbuf: Vec::new(),
            rpos: 0,
        }
    }

    pub fn is_seekable(&self) -> bool {
        matches!(self.backend, Backend::File(_) | Backend::Memory { .. })
    }

    /// Puts unread read-ahead back, so the file position is the logical one
    fn sync(&mut self) -> io::Result<()> {
        let left = self.rbuf.len() - self.rpos;
        if left > 0
            && let Backend::File(f) = &mut self.backend
        {
            f.seek(SeekFrom::Current(-(left as i64)))?;
        }
        self.rbuf.clear();
        self.rpos = 0;
        Ok(())
    }

    /// Reads from the underlying stream into `buf`, past any read-ahead
    fn raw_read(&mut self, buf: &mut [u8]) -> Result<usize, IoFail> {
        if self.rpos < self.rbuf.len() {
            let n = buf.len().min(self.rbuf.len() - self.rpos);
            buf[..n].copy_from_slice(&self.rbuf[self.rpos..self.rpos + n]);
            self.rpos += n;
            return Ok(n);
        }
        match &mut self.backend {
            Backend::File(f) => Ok(f.read(buf)?),
            Backend::Stdin => Ok(io::stdin().lock().read(buf)?),
            Backend::Memory { data, pos } => {
                if *pos >= data.len() {
                    return Ok(0);
                }
                let n = buf.len().min(data.len() - *pos);
                buf[..n].copy_from_slice(&data[*pos..*pos + n]);
                *pos += n;
                Ok(n)
            }
            Backend::Stdout | Backend::Stderr | Backend::Output => Err(bad_fd()),
            Backend::Dir { .. } => Err(IoFail { errno: 21, msg: "Is a directory".into() }),
        }
    }

    fn check_readable(&self) -> Result<(), IoFail> {
        if !self.readable || matches!(self.backend, Backend::Stdout | Backend::Stderr | Backend::Output) {
            return Err(bad_fd());
        }
        Ok(())
    }

    /// fread: up to `n` bytes
    pub fn read(&mut self, n: usize) -> Result<Vec<u8>, IoFail> {
        self.check_readable()?;
        // Grown as data arrives, so a huge length costs nothing up front
        let mut out = Vec::new();
        let mut got = 0;
        while got < n {
            if got >= READ_LIMIT {
                return Err(too_big());
            }
            let want = (n - got).min(1 << 20);
            out.resize(got + want, 0);
            let k = self.raw_read(&mut out[got..])?;
            if k == 0 {
                break;
            }
            got += k;
            // Like php-src, only plain files keep reading for the rest
            if !matches!(self.backend, Backend::File(_) | Backend::Memory { .. }) {
                break;
            }
        }
        out.truncate(got);
        if got < n {
            self.eof = true;
        }
        self.count += got as u64;
        Ok(out)
    }

    /// One more byte of read-ahead, for line reading
    fn next_byte(&mut self) -> Result<Option<u8>, IoFail> {
        if self.rpos >= self.rbuf.len() {
            if let Backend::Memory { data, pos } = &mut self.backend {
                if *pos < data.len() {
                    *pos += 1;
                    return Ok(Some(data[*pos - 1]));
                }
                return Ok(None);
            }
            self.rbuf.resize(CHUNK, 0);
            self.rpos = 0;
            let mut buf = std::mem::take(&mut self.rbuf);
            let r = match &mut self.backend {
                Backend::File(f) => f.read(&mut buf).map_err(IoFail::from),
                Backend::Stdin => io::stdin().lock().read(&mut buf).map_err(IoFail::from),
                Backend::Dir { .. } => Err(IoFail { errno: 21, msg: "Is a directory".into() }),
                _ => Err(bad_fd()),
            };
            let n = match r {
                Ok(n) => n,
                Err(e) => {
                    self.rbuf.clear();
                    return Err(e);
                }
            };
            buf.truncate(n);
            self.rbuf = buf;
            if n == 0 {
                return Ok(None);
            }
        }
        self.rpos += 1;
        Ok(Some(self.rbuf[self.rpos - 1]))
    }

    /// fgets: a line with its newline, at most `max` bytes. None at the end
    pub fn read_line(&mut self, max: Option<usize>) -> Result<Option<Vec<u8>>, IoFail> {
        self.check_readable()?;
        let mut line = Vec::new();
        loop {
            if max.is_some_and(|m| line.len() >= m) {
                break;
            }
            if line.len() >= READ_LIMIT {
                return Err(too_big());
            }
            match self.next_byte()? {
                Some(b) => {
                    line.push(b);
                    if b == b'\n' {
                        break;
                    }
                }
                None => {
                    self.eof = true;
                    break;
                }
            }
        }
        self.count += line.len() as u64;
        Ok(if line.is_empty() { None } else { Some(line) })
    }

    /// Everything from here to the end
    pub fn read_all(&mut self) -> Result<Vec<u8>, IoFail> {
        self.check_readable()?;
        let mut out = self.rbuf[self.rpos..].to_vec();
        self.rbuf.clear();
        self.rpos = 0;
        let limit = (READ_LIMIT + 1) as u64;
        match &mut self.backend {
            Backend::File(f) => {
                f.take(limit).read_to_end(&mut out)?;
            }
            Backend::Stdin => {
                io::stdin().lock().take(limit).read_to_end(&mut out)?;
            }
            Backend::Memory { data, pos } => {
                out.extend_from_slice(&data[(*pos).min(data.len())..]);
                *pos = (*pos).max(data.len());
            }
            Backend::Dir { .. } => return Err(IoFail { errno: 21, msg: "Is a directory".into() }),
            _ => return Err(bad_fd()),
        }
        if out.len() > READ_LIMIT {
            return Err(too_big());
        }
        self.eof = true;
        self.count += out.len() as u64;
        Ok(out)
    }

    pub fn write(&mut self, sink: &mut dyn Sink, bytes: &[u8]) -> Result<usize, IoFail> {
        if !self.writable {
            // A read-only memory stream refuses quietly
            if matches!(self.backend, Backend::Memory { .. }) {
                return Err(IoFail { errno: 0, msg: String::new() });
            }
            return Err(bad_fd());
        }
        self.sync()?;
        match &mut self.backend {
            Backend::File(f) => {
                f.write_all(bytes)?;
            }
            Backend::Memory { data, pos } => {
                if self.append {
                    *pos = data.len();
                }
                let end = *pos + bytes.len();
                if data.len() < end {
                    data.resize(end, 0);
                }
                data[*pos..end].copy_from_slice(bytes);
                *pos = end;
            }
            Backend::Stdout | Backend::Output => sink.out(bytes),
            Backend::Stderr => {
                sink.flush_out();
                let mut e = io::stderr().lock();
                e.write_all(bytes)?;
                e.flush()?;
            }
            Backend::Stdin | Backend::Dir { .. } => return Err(bad_fd()),
        }
        self.count += bytes.len() as u64;
        Ok(bytes.len())
    }

    pub fn tell(&mut self) -> Option<u64> {
        let pending = (self.rbuf.len() - self.rpos) as u64;
        match &mut self.backend {
            Backend::File(f) => f.stream_position().ok().map(|p| p - pending),
            Backend::Memory { pos, .. } => Some(*pos as u64),
            Backend::Dir { .. } => None,
            _ => Some(self.count),
        }
    }

    /// fseek's whence: 0 SEEK_SET, 1 SEEK_CUR, 2 SEEK_END. False when the
    /// position would be negative or the stream cannot seek
    pub fn seek(&mut self, offset: i64, whence: i64) -> bool {
        let cur = self.tell();
        if self.sync().is_err() {
            return false;
        }
        let ok = match &mut self.backend {
            Backend::File(f) => {
                let target = match whence {
                    0 => (offset >= 0).then_some(SeekFrom::Start(offset as u64)),
                    1 => cur.and_then(|c| c.checked_add_signed(offset)).map(SeekFrom::Start),
                    2 => Some(SeekFrom::End(offset)),
                    _ => None,
                };
                target.is_some_and(|t| f.seek(t).is_ok())
            }
            Backend::Memory { data, pos } => {
                let base = match whence {
                    0 => 0,
                    1 => *pos as i64,
                    2 => data.len() as i64,
                    _ => return false,
                };
                match base.checked_add(offset) {
                    Some(p) if p >= 0 => {
                        *pos = p as usize;
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        };
        if ok {
            self.eof = false;
        }
        ok
    }

    pub fn truncate(&mut self, size: u64) -> bool {
        if !self.writable || self.sync().is_err() {
            return false;
        }
        match &mut self.backend {
            Backend::File(f) => f.set_len(size).is_ok(),
            Backend::Memory { data, .. } => {
                data.resize(size as usize, 0);
                true
            }
            _ => false,
        }
    }

    pub fn flush(&mut self, sink: &mut dyn Sink) -> bool {
        match &mut self.backend {
            Backend::File(f) => f.flush().is_ok(),
            Backend::Stdout | Backend::Output => {
                sink.flush_out();
                true
            }
            _ => true,
        }
    }

    /// The size for fstat
    pub fn size(&mut self) -> Option<u64> {
        match &mut self.backend {
            Backend::File(f) => f.metadata().ok().map(|m| m.len()),
            Backend::Memory { data, .. } => Some(data.len() as u64),
            _ => None,
        }
    }

    pub fn file(&self) -> Option<&File> {
        match &self.backend {
            Backend::File(f) => Some(f),
            _ => None,
        }
    }

    /// The next directory entry, for readdir
    pub fn next_entry(&mut self) -> Option<Vec<u8>> {
        match &mut self.backend {
            Backend::Dir { entries, pos } => {
                let e = entries.get(*pos).cloned();
                *pos += 1;
                e
            }
            _ => None,
        }
    }

    pub fn rewind_dir(&mut self) {
        if let Backend::Dir { pos, .. } = &mut self.backend {
            *pos = 0;
        }
    }
}

/// (readable, writable, append) for an fopen mode, or None if it is not one
pub fn mode_flags(mode: &str) -> Option<(bool, bool, bool)> {
    let b = mode.as_bytes();
    let plus = b.get(1..).is_some_and(|r| r.contains(&b'+'));
    Some(match b.first()? {
        b'r' => (true, plus, false),
        b'w' | b'x' | b'c' => (plus, true, false),
        b'a' => (plus, true, true),
        _ => return None,
    })
}
