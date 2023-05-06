use crate::error::ReadExactError;

#[cfg(feature = "std")]
use crate::{ErrorKind, IoError};

pub use embedded_io::Io as IoBase;
pub use embedded_io::SeekFrom;

pub use embedded_io::blocking::{Read, Seek, Write};

/// A wrapper struct for types that have implementations for `std::io` traits.
///
/// `Read`, `Write`, `Seek` traits from this crate are implemented for this type if
/// corresponding types from `std::io` are implemented by the inner instance.
#[cfg(feature = "std")]
pub struct StdIoWrapper<T> {
    inner: T,
}

#[cfg(feature = "std")]
impl<T> StdIoWrapper<T> {
    /// Creates a new `StdIoWrapper` instance that wraps the provided `inner` instance.
    pub fn new(inner: T) -> Self {
        Self { inner }
    }

    /// Returns inner struct
    pub fn into_inner(self) -> T {
        self.inner
    }
}

#[cfg(feature = "std")]
#[derive(Debug)]
pub struct StdErrWrapper {}

#[cfg(feature = "std")]
#[derive(Debug)]
pub enum StdSeekPosWrapper {
    Start(u64),
    End(i64),
    Current(i64),
}

#[cfg(feature = "std")]
impl From<embedded_io::SeekFrom> for StdSeekPosWrapper {
    fn from(pos: embedded_io::SeekFrom) -> Self {
        match pos {
            embedded_io::SeekFrom::Start(pos) => Self::Start(pos),
            embedded_io::SeekFrom::End(pos) => Self::End(pos),
            embedded_io::SeekFrom::Current(pos) => Self::Current(pos),
        }
    }
}

#[cfg(feature = "std")]
impl From<std::io::SeekFrom> for StdSeekPosWrapper {
    fn from(pos: std::io::SeekFrom) -> Self {
        match pos {
            std::io::SeekFrom::Start(pos) => Self::Start(pos),
            std::io::SeekFrom::End(pos) => Self::End(pos),
            std::io::SeekFrom::Current(pos) => Self::Current(pos),
        }
    }
}

#[cfg(feature = "std")]
impl Into<std::io::SeekFrom> for StdSeekPosWrapper {
    fn into(self) -> std::io::SeekFrom {
        match self {
            Self::Start(pos) => std::io::SeekFrom::Start(pos),
            Self::End(pos) => std::io::SeekFrom::End(pos),
            Self::Current(pos) => std::io::SeekFrom::Current(pos),
        }
    }
}

#[cfg(feature = "std")]
impl Into<embedded_io::SeekFrom> for StdSeekPosWrapper {
    fn into(self) -> embedded_io::SeekFrom {
        match self {
            Self::Start(pos) => embedded_io::SeekFrom::Start(pos),
            Self::End(pos) => embedded_io::SeekFrom::End(pos),
            Self::Current(pos) => embedded_io::SeekFrom::Current(pos),
        }
    }
}

#[cfg(feature = "std")]
impl From<std::io::Error> for StdErrWrapper {
    fn from(_: std::io::Error) -> Self {
        Self {}
    }
}

#[cfg(feature = "std")]
impl Into<std::io::Error> for StdErrWrapper {
    fn into(self) -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::Other, StdErrWrapper {})
    }
}

#[cfg(feature = "std")]
impl From<ReadExactError<StdErrWrapper>> for StdErrWrapper {
    fn from(_: ReadExactError<StdErrWrapper>) -> Self {
        Self {}
    }
}

#[cfg(feature = "std")]
impl IoError for StdErrWrapper {
    fn kind(&self) -> ErrorKind {
        ErrorKind::Other
    }
}

#[cfg(feature = "std")]
impl<T> IoBase for StdIoWrapper<T> {
    type Error = StdErrWrapper;
}

#[cfg(feature = "std")]
impl<T: std::io::Read> Read for StdIoWrapper<T> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        Ok(self.inner.read(buf)?)
    }
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), ReadExactError<Self::Error>>
    where
        Self::Error: From<ReadExactError<Self::Error>>,
    {
        match self.inner.read_exact(buf) {
            Ok(()) => Ok(()),
            Err(error) => match error.kind() {
                std::io::ErrorKind::UnexpectedEof => Err(ReadExactError::UnexpectedEof),
                _ => Err(ReadExactError::Other(error.into())),
            },
        }
    }
}

#[cfg(feature = "std")]
impl<T: std::io::Write> Write for StdIoWrapper<T> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        Ok(self.inner.write(buf)?)
    }

    fn write_all(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
        Ok(self.inner.write_all(buf)?)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(self.inner.flush()?)
    }
}

#[cfg(feature = "std")]
impl<T: std::io::Seek> Seek for StdIoWrapper<T> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        Ok(self.inner.seek(StdSeekPosWrapper::from(pos).into())?)
    }
}

#[cfg(feature = "std")]
impl<T> From<T> for StdIoWrapper<T> {
    fn from(from: T) -> Self {
        Self::new(from)
    }
}

pub(crate) trait ReadLeExt {
    type Error;
    fn read_u8(&mut self) -> Result<u8, Self::Error>;
    fn read_u16_le(&mut self) -> Result<u16, Self::Error>;
    fn read_u32_le(&mut self) -> Result<u32, Self::Error>;
}

impl<T: Read> ReadLeExt for T
where
    <T as IoBase>::Error: From<ReadExactError<<T as IoBase>::Error>>,
{
    type Error = <Self as IoBase>::Error;

    fn read_u8(&mut self) -> Result<u8, Self::Error> {
        let mut buf = [0_u8; 1];
        self.read_exact(&mut buf)?;
        Ok(buf[0])
    }

    fn read_u16_le(&mut self) -> Result<u16, Self::Error> {
        let mut buf = [0_u8; 2];
        self.read_exact(&mut buf)?;
        Ok(u16::from_le_bytes(buf))
    }

    fn read_u32_le(&mut self) -> Result<u32, Self::Error> {
        let mut buf = [0_u8; 4];
        self.read_exact(&mut buf)?;
        Ok(u32::from_le_bytes(buf))
    }
}

pub(crate) trait WriteLeExt {
    type Error;
    fn write_u8(&mut self, n: u8) -> Result<(), Self::Error>;
    fn write_u16_le(&mut self, n: u16) -> Result<(), Self::Error>;
    fn write_u32_le(&mut self, n: u32) -> Result<(), Self::Error>;
}

impl<T: Write> WriteLeExt for T {
    type Error = <Self as IoBase>::Error;

    fn write_u8(&mut self, n: u8) -> Result<(), Self::Error> {
        self.write_all(&[n])
    }

    fn write_u16_le(&mut self, n: u16) -> Result<(), Self::Error> {
        self.write_all(&n.to_le_bytes())
    }

    fn write_u32_le(&mut self, n: u32) -> Result<(), Self::Error> {
        self.write_all(&n.to_le_bytes())
    }
}
