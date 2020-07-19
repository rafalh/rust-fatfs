#[cfg(all(not(feature = "std"), feature = "alloc"))]
use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Other,
    InvalidInput,
    UnexpectedEof,
    NotFound,
    AlreadyExists,
    WriteZero,
}

#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    msg: &'static str,
}

impl Error {
    pub fn new(kind: ErrorKind, msg: &'static str) -> Self {
        Error { kind, msg }
    }
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.msg)
    }
}

pub type Result<T> = core::result::Result<T, Error>;

pub trait Read {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
    fn read_exact(&mut self, mut buf: &mut [u8]) -> Result<()> {
        while !buf.is_empty() {
            match self.read(buf) {
                Ok(0) => break,
                Ok(n) => {
                    let tmp = buf;
                    buf = &mut tmp[n..];
                }
                //Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        if !buf.is_empty() {
            Err(Error::new(ErrorKind::UnexpectedEof, "failed to fill whole buffer"))
        } else {
            Ok(())
        }
    }
}

pub trait Write {
    fn write(&mut self, buf: &[u8]) -> Result<usize>;
    fn write_all(&mut self, mut buf: &[u8]) -> Result<()> {
        while !buf.is_empty() {
            match self.write(buf) {
                Ok(0) => {
                    return Err(Error::new(ErrorKind::WriteZero, "failed to write whole buffer"));
                }
                Ok(n) => buf = &buf[n..],
                //Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<()>;
}

pub enum SeekFrom {
    Start(u64),
    End(i64),
    Current(i64),
}

pub trait Seek {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64>;
}

#[cfg(feature = "std")]
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::Other => Error::new(ErrorKind::Other, "other"),
            std::io::ErrorKind::InvalidInput => Error::new(ErrorKind::InvalidInput, "invalid input"),
            std::io::ErrorKind::UnexpectedEof => Error::new(ErrorKind::UnexpectedEof, "unexpected eof"),
            std::io::ErrorKind::NotFound => Error::new(ErrorKind::NotFound, "not found"),
            std::io::ErrorKind::AlreadyExists => Error::new(ErrorKind::AlreadyExists, "already exists"),
            std::io::ErrorKind::WriteZero => Error::new(ErrorKind::WriteZero, "write zero"),
            _ => Error::new(ErrorKind::Other, "unknown"),
        }
    }
}

#[cfg(feature = "std")]
impl From<Error> for std::io::Error {
    fn from(error: Error) -> Self {
        match error.kind() {
            ErrorKind::Other => std::io::Error::new(std::io::ErrorKind::Other, "other"),
            ErrorKind::InvalidInput => std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid input"),
            ErrorKind::UnexpectedEof => std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "unexpected eof"),
            ErrorKind::NotFound => std::io::Error::new(std::io::ErrorKind::NotFound, "not found"),
            ErrorKind::AlreadyExists => std::io::Error::new(std::io::ErrorKind::AlreadyExists, "already exists"),
            ErrorKind::WriteZero => std::io::Error::new(std::io::ErrorKind::WriteZero, "write zero"),
        }
    }
}

#[cfg(feature = "std")]
impl Into<std::io::SeekFrom> for SeekFrom {
    fn into(self) -> std::io::SeekFrom {
        match self {
            SeekFrom::Start(n) => std::io::SeekFrom::Start(n),
            SeekFrom::End(n) => std::io::SeekFrom::End(n),
            SeekFrom::Current(n) => std::io::SeekFrom::Current(n),
        }
    }
}

#[cfg(feature = "std")]
impl From<std::io::SeekFrom> for SeekFrom {
    fn from(from: std::io::SeekFrom) -> Self {
        match from {
            std::io::SeekFrom::Start(n) => SeekFrom::Start(n),
            std::io::SeekFrom::End(n) => SeekFrom::End(n),
            std::io::SeekFrom::Current(n) => SeekFrom::Current(n),
        }
    }
}

#[cfg(feature = "std")]
pub struct StdIoWrapper<T> {
    inner: T,
}

#[cfg(feature = "std")]
impl<T> StdIoWrapper<T> {
    pub fn new(inner: T) -> Self {
        StdIoWrapper { inner }
    }
}

#[cfg(feature = "std")]
impl<T: std::io::Read> Read for StdIoWrapper<T> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        Ok(self.inner.read(buf)?)
    }
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        Ok(self.inner.read_exact(buf)?)
    }
}

#[cfg(feature = "std")]
impl<T: std::io::Write> Write for StdIoWrapper<T> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        Ok(self.inner.write(buf)?)
    }

    fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        Ok(self.inner.write_all(buf)?)
    }

    fn flush(&mut self) -> Result<()> {
        Ok(self.inner.flush()?)
    }
}

#[cfg(feature = "std")]
impl<T: std::io::Seek> Seek for StdIoWrapper<T> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        Ok(self.inner.seek(pos.into())?)
    }
}


#[cfg(feature = "std")]
impl<T> From<T> for StdIoWrapper<T> {
    fn from(from: T) -> Self {
        StdIoWrapper::new(from)
    }
}

pub(crate) struct Cursor<T> {
    inner: T,
    pos: usize
}

impl<T> Cursor<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            pos: 0,
        }
    }
}

impl<T> Read for Cursor<T> where T: AsRef<[u8]> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let n = core::cmp::min(buf.len(), self.inner.as_ref().len() - self.pos);
        let new_pos = self.pos + n;
        buf[..n].copy_from_slice(&self.inner.as_ref()[self.pos..new_pos]);
        self.pos = new_pos;
        Ok(n)
    }
}

#[cfg(feature = "alloc")]
impl Write for Cursor<Vec<u8>> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        let len = self.inner.len();
        if len < self.pos {
            self.inner.resize(self.pos, 0);
        }
        let space = self.inner.len() - self.pos;
        let (left, right) = buf.split_at(core::cmp::min(space, buf.len()));
        self.inner[self.pos..self.pos + left.len()].copy_from_slice(left);
        self.inner.extend_from_slice(right);
        self.pos += buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

fn create_eof_error() -> Error {
    Error::new(ErrorKind::UnexpectedEof, "requested offset is past the current end of the file")
}

impl<T> Seek for Cursor<T> where T: AsRef<[u8]> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        match pos {
            SeekFrom::Start(n) => {
                if n > self.inner.as_ref().len() as u64 {
                    return Err(create_eof_error());
                } else {
                    self.pos = n as usize
                }
            },
            SeekFrom::End(n) => {
                if n > self.inner.as_ref().len() as i64 {
                    return Err(create_eof_error());
                } else {
                    self.pos = self.inner.as_ref().len() - n as usize
                }
            },
            SeekFrom::Current(n) => {
                if self.pos as i64 + n > self.inner.as_ref().len() as i64 {
                    return Err(create_eof_error());
                } else {
                    self.pos += n as usize
                }
            },
        }
        Ok(self.pos as u64)
    }
}

pub(crate) trait ReadLeExt {
    fn read_u8(&mut self) -> Result<u8>;
    fn read_u16_le(&mut self) -> Result<u16>;
    fn read_u32_le(&mut self) -> Result<u32>;
}

impl<T: Read> ReadLeExt for T {
    fn read_u8(&mut self) -> Result<u8> {
        let mut buf = [0u8; 1];
        self.read_exact(&mut buf)?;
        Ok(buf[0])
    }

    fn read_u16_le(&mut self) -> Result<u16> {
        let mut buf = [0u8; 2];
        self.read_exact(&mut buf)?;
        Ok(u16::from_le_bytes(buf))
    }

    fn read_u32_le(&mut self) -> Result<u32> {
        let mut buf = [0u8; 4];
        self.read_exact(&mut buf)?;
        Ok(u32::from_le_bytes(buf))
    }
}

pub(crate) trait WriteLeExt {
    fn write_u8(&mut self, n: u8) -> Result<()>;
    fn write_u16_le(&mut self, n: u16) -> Result<()>;
    fn write_u32_le(&mut self, n: u32) -> Result<()>;
}

impl<T: Write> WriteLeExt for T {
    fn write_u8(&mut self, n: u8) -> Result<()> {
        self.write_all(&[n])
    }

    fn write_u16_le(&mut self, n: u16) -> Result<()> {
        self.write_all(&n.to_le_bytes())
    }

    fn write_u32_le(&mut self, n: u32) -> Result<()> {
        self.write_all(&n.to_le_bytes())
    }
}
