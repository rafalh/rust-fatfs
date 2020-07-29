use crate::error::IoError;
#[cfg(all(not(feature = "std"), feature = "alloc"))]
use alloc::vec::Vec;

/// Provides IO error as an associated type.
///
/// Must be implemented for all types that also implement at least one of the following traits: `Read`, `Write`,
/// `Seek`.
pub trait IoBase {
    /// Type of errors returned by input/output operations.
    type Error: IoError;
}

/// The `Read` trait allows for reading bytes from a source.
///
/// It is based on the `std::io::Read` trait.
pub trait Read: IoBase {
    /// Pull some bytes from this source into the specified buffer, returning how many bytes were read.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error>;

    /// Read the exact number of bytes required to fill `buf`.
    ///
    /// This function reads as many bytes as necessary to completely fill the specified buffer `buf`.
    /// If this function encounters an end of file before completely filling the buffer, it returns an error
    /// instantiated by a call to `IoError::new_unexpected_eof_error`. The contents of buf are unspecified in this
    /// case.
    fn read_exact(&mut self, mut buf: &mut [u8]) -> Result<(), Self::Error> {
        while !buf.is_empty() {
            match self.read(buf) {
                Ok(0) => break,
                Ok(n) => {
                    let tmp = buf;
                    buf = &mut tmp[n..];
                }
                Err(ref e) if e.is_interrupted() => {}
                Err(e) => return Err(e),
            }
        }
        if buf.is_empty() {
            Ok(())
        } else {
            debug!("failed to fill whole buffer in read_exact");
            Err(Self::Error::new_unexpected_eof_error())
        }
    }
}

/// The `Write` trait allows for writing bytes into the sink.
///
/// It is based on the `std::io::Write` trait.
pub trait Write: IoBase {
    /// Write a buffer into this writer, returning how many bytes were written.
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error>;

    /// Attempts to write an entire buffer into this writer.
    ///
    /// This method will continuously call `write` until there is no more data to be written or an error is returned.
    /// Errors for which `IoError::is_interrupted` method returns true are being skipped. This method will not return
    /// until the entire buffer has been successfully written or such an error occurs.
    /// If `write` returns 0 before the entire buffer has been written this method will return an error instantiated by
    /// a call to `IoError::new_write_zero_error`.
    fn write_all(&mut self, mut buf: &[u8]) -> Result<(), Self::Error> {
        while !buf.is_empty() {
            match self.write(buf) {
                Ok(0) => {
                    debug!("failed to write whole buffer in write_all");
                    return Err(Self::Error::new_write_zero_error());
                }
                Ok(n) => buf = &buf[n..],
                Err(ref e) if e.is_interrupted() => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), Self::Error>;
}

/// Enumeration of possible methods to seek within an I/O object.
///
/// It is based on the `std::io::SeekFrom` enum.
pub enum SeekFrom {
    /// Sets the offset to the provided number of bytes.
    Start(u64),
    /// Sets the offset to the size of this object plus the specified number of bytes.
    End(i64),
    /// Sets the offset to the current position plus the specified number of bytes.
    Current(i64),
}

/// The `Seek` trait provides a cursor which can be moved within a stream of bytes.
///
/// It is based on the `std::io::Seek` trait.
pub trait Seek: IoBase {
    /// Seek to an offset, in bytes, in a stream.
    ///
    /// A seek beyond the end of a stream or to a negative position is not allowed.
    ///
    /// If the seek operation completed successfully, this method returns the new position from the start of the
    /// stream. That position can be used later with `SeekFrom::Start`.
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error>;
}

#[cfg(feature = "std")]
impl From<SeekFrom> for std::io::SeekFrom {
    fn from(from: SeekFrom) -> Self {
        match from {
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
}

#[cfg(feature = "std")]
impl<T> IoBase for StdIoWrapper<T> {
    type Error = std::io::Error;
}

#[cfg(feature = "std")]
impl<T: std::io::Read> Read for StdIoWrapper<T> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        Ok(self.inner.read(buf)?)
    }
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), Self::Error> {
        Ok(self.inner.read_exact(buf)?)
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
        Ok(self.inner.seek(pos.into())?)
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

impl<T: Read> ReadLeExt for T {
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
