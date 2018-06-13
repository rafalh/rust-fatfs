extern crate fatfs;

use std::{cmp, fs, io};
use std::io::{prelude::*, SeekFrom, ErrorKind};
use fatfs::{FileSystem, FsOptions, BufStream};

trait ReadWriteSeek: Read + Write + Seek {}
impl<T> ReadWriteSeek for T where T: Read + Write + Seek {}

// File wrapper for accessing part of file
#[derive(Clone)]
pub(crate) struct Partition<T: ReadWriteSeek> {
    inner: T,
    start_offset: u64,
    current_offset: u64,
    size: u64,
}

impl <T: ReadWriteSeek> Partition<T> {
    pub(crate) fn new(mut inner: T, start_offset: u64, size: u64) -> io::Result<Self> {
        inner.seek(SeekFrom::Start(start_offset))?;
        Ok(Self {
            start_offset, size, inner,
            current_offset: 0,
        })
    }
}

impl <T: ReadWriteSeek> Read for Partition<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let max_read_size = cmp::min((self.size - self.current_offset) as usize, buf.len());
        let bytes_read = self.inner.read(&mut buf[..max_read_size])?;
        self.current_offset += bytes_read as u64;
        Ok(bytes_read)
    }
}

impl <T: ReadWriteSeek> Write for Partition<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let max_write_size = cmp::min((self.size - self.current_offset) as usize, buf.len());
        let bytes_written = self.inner.write(&buf[..max_write_size])?;
        self.current_offset += bytes_written as u64;
        Ok(bytes_written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl <T: ReadWriteSeek> Seek for Partition<T> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_offset = match pos {
            SeekFrom::Current(x) => self.current_offset as i64 + x,
            SeekFrom::Start(x) => x as i64,
            SeekFrom::End(x) => self.size as i64 + x,
        };
        if new_offset < 0 || new_offset as u64 > self.size {
            Err(io::Error::new(ErrorKind::InvalidInput, "invalid seek"))
        } else {
            self.inner.seek(SeekFrom::Start(self.start_offset + new_offset as u64))?;
            self.current_offset = new_offset as u64;
            Ok(self.current_offset)
        }
    }
}

fn main() -> io::Result<()> {
    // Open disk image
    let file = fs::File::open("resources/fat32.img")?;
    // Provide sample partition localization. In real application it should be read from MBR/GPT.
    let first_lba = 0;
    let last_lba = 10000;
    // Create partition using provided start address and size in bytes
    let partition = Partition::<fs::File>::new(file, first_lba, last_lba - first_lba + 1)?;
    // Create buffered stream to optimize file access
    let buf_rdr = BufStream::new(partition);
    // Finally initialize filesystem struct using provided partition
    let fs = FileSystem::new(buf_rdr, FsOptions::new())?;
    // Read and display volume label
    println!("Volume Label: {}", fs.volume_label());
    // other operations...
    Ok(())
}
