use std::io::prelude::*;
use std::io;
use std::cmp;

pub trait ReadSeek: Read + Seek {}
impl<T> ReadSeek for T where T: Read + Seek {}

pub trait ReadWriteSeek: Read + Write + Seek {}
impl<T> ReadWriteSeek for T where T: Read + Write + Seek {}

const BUF_SIZE: usize = 512;

pub struct BufStream<T: Read+Write+Seek>  {
    inner: T,
    buf: [u8; BUF_SIZE],
    buf_offset: usize,
    buf_len: usize,
    dirty: bool,
    inner_offset: usize,
}

impl<T: Read+Write+Seek> BufStream<T> {
    pub fn new(inner: T) -> Self {
        BufStream::<T> {
            inner,
            buf: [0; BUF_SIZE],
            buf_offset: 0,
            buf_len: 0,
            dirty: false,
            inner_offset: 0,
        }
    }
}

impl<T: Read+Write+Seek> Read for BufStream<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut num_done = 0;
        let mut num_todo = buf.len();
        let mut eof = false;
        loop {
            let num_ready = cmp::min(num_todo, self.buf_len - self.buf_offset);
            buf[num_done..num_done+num_ready].clone_from_slice(&self.buf[self.buf_offset..self.buf_offset+num_ready]);
            self.buf_offset += num_ready;
            num_done += num_ready;
            num_todo -= num_ready;
            if eof || num_todo == 0 {
                break;
            }
            if num_todo > BUF_SIZE {
                let num_read = self.inner.read(&mut buf[num_done..])?;
                num_done += num_read;
                num_todo -= num_read;
                let num_copy = cmp::min(BUF_SIZE, num_done);
                self.buf[..num_copy].clone_from_slice(&buf[num_done - num_copy..]);
                self.buf_len = num_copy;
                self.buf_offset = num_copy;
                self.inner_offset = num_copy;
                eof = true;
            } else {
                if self.inner_offset != self.buf_offset {
                    self.inner.seek(io::SeekFrom::Current((self.buf_offset - self.inner_offset) as i64))?;
                }
                self.buf_len = self.inner.read(&mut self.buf)?;
                self.buf_offset = 0;
                self.inner_offset = self.buf_len;
                eof = true;
            }
        }
        Ok(num_done)
    }
}

impl<T: Read+Write+Seek> BufStream<T> {
    fn write_buf(&mut self) -> io::Result<()> {
        if self.dirty {
            if self.inner_offset > 0 {
                self.inner.seek(io::SeekFrom::Current(-(self.inner_offset as i64)))?;
            }
            self.inner.write(&self.buf[..self.buf_len])?;
            self.inner_offset = self.buf_len;
            self.dirty = false;
        }
        Ok(())
    }
}

impl<T: Read+Write+Seek> Write for BufStream<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut num_done = 0;
        let mut num_todo = buf.len();
        
        loop {
            let num_ready = cmp::min(num_todo, BUF_SIZE - self.buf_offset);
            self.buf[self.buf_offset..self.buf_offset+num_ready].clone_from_slice(&buf[num_done..num_done+num_ready]);
            self.buf_offset += num_ready;
            self.buf_len = cmp::max(self.buf_len, self.buf_offset);
            self.dirty = num_ready > 0;
            num_done += num_ready;
            num_todo -= num_ready;
            if num_todo == 0 {
                break;
            }
            self.write_buf()?;
            self.buf_offset = 0;
            self.buf_len = 0;
            self.inner_offset = 0;
        }
        Ok(num_done)
    }
    
    fn flush(&mut self) -> io::Result<()> {
        self.write_buf()?;
        self.inner.flush()
    }
}

impl<T: Read+Write+Seek> Seek for BufStream<T> {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        // FIXME: reuse buffer
        let new_pos = match pos {
            io::SeekFrom::Current(x) => io::SeekFrom::Current(x - self.inner_offset as i64 + self.buf_offset as i64),
            _ => pos,
        };
        self.buf_offset = 0;
        self.buf_len = 0;
        self.inner_offset = 0;
        self.inner.seek(new_pos)
    }
}

impl<T: Read+Write+Seek> Drop for BufStream<T> {
    fn drop(&mut self) {
        self.flush().expect("flush failed!");
    }
}
