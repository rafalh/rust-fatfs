use io::prelude::*;
use io;
use core::cmp;

pub trait ReadSeek: Read + Seek {}
impl<T> ReadSeek for T where T: Read + Seek {}

pub trait ReadWriteSeek: Read + Write + Seek {}
impl<T> ReadWriteSeek for T where T: Read + Write + Seek {}

const BUF_SIZE: usize = 512;

pub struct BufStream<T: Read+Write+Seek>  {
    inner: T,
    buf: [u8; BUF_SIZE],
    len: usize,
    pos: usize,
    write: bool,
}

/// The BufStream struct adds buffering to underlying file or device.
///
/// It's basically composition of BufReader and BufWritter.
impl<T: Read+Write+Seek> BufStream<T> {
    /// Creates new BufStream object for given stream.
    pub fn new(inner: T) -> Self {
        BufStream::<T> {
            inner,
            buf: [0; BUF_SIZE],
            pos: 0,
            len: 0,
            write: false,
        }
    }

    fn flush_buf(&mut self) -> io::Result<()> {
        if self.write {
            self.inner.write_all(&self.buf[..self.pos])?;
            self.pos = 0;
        }
        Ok(())
    }

    fn make_reader(&mut self) -> io::Result<()> {
        if self.write {
            self.flush_buf()?;
            self.write = false;
            self.len = 0;
            self.pos = 0;
        }
        Ok(())
    }

    fn make_writter(&mut self) -> io::Result<()> {
        if !self.write {
            self.inner.seek(io::SeekFrom::Current(-(self.len as i64 - self.pos as i64)))?;
            self.write = true;
            self.len = 0;
            self.pos = 0;
        }
        Ok(())
    }
}

impl<T: Read+Write+Seek> BufRead for BufStream<T> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.make_reader()?;
        if self.pos >= self.len {
            self.len = self.inner.read(&mut self.buf)?;
            self.pos = 0;
        }
        Ok(&self.buf[self.pos..self.len])
    }

    fn consume(&mut self, amt: usize) {
        self.pos = cmp::min(self.pos + amt, self.len);
    }
}

impl<T: Read+Write+Seek> Read for BufStream<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Make sure we are in read mode
        self.make_reader()?;
        // Check if this read is bigger than buffer size
        if self.pos == self.len && buf.len() >= BUF_SIZE {
            return self.inner.read(buf);
        }
        let nread = {
            let mut rem = self.fill_buf()?;
            rem.read(buf)?
        };
        self.consume(nread);
        Ok(nread)
    }
}

impl<T: Read+Write+Seek> Write for BufStream<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Make sure we are in write mode
        self.make_writter()?;
        if self.pos + buf.len() > BUF_SIZE {
            self.flush_buf()?;
            if buf.len() >= BUF_SIZE {
                return self.inner.write(buf);
            }
        }
        let written = (&mut self.buf[self.pos..]).write(buf)?;
        self.pos += written;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flush_buf()?;
        self.inner.flush()
    }
}

impl<T: Read+Write+Seek> Seek for BufStream<T> {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        self.flush_buf()?;
        let new_pos = match pos {
            io::SeekFrom::Current(x) => io::SeekFrom::Current(x - (self.len as i64 - self.pos as i64)),
            _ => pos,
        };
        self.pos = 0;
        self.len = 0;
        self.inner.seek(new_pos)
    }
}

impl<T: Read+Write+Seek> Drop for BufStream<T> {
    fn drop(&mut self) {
        match self.flush() {
            Err(err) => error!("flush failed {}", err),
            _ => {},
        }
    }
}
