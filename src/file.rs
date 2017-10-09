use std::cmp;
use std::io::prelude::*;
use std::io::{SeekFrom, ErrorKind};
use std::io;

use fs::FileSystemRef;
use dir::{FileEntryInfo, DateTime};

#[derive(Clone)]
pub struct File<'a, 'b: 'a> {
    first_cluster: Option<u32>,
    current_cluster: Option<u32>,
    offset: u32,
    entry: Option<FileEntryInfo>,
    entry_dirty: bool,
    fs: FileSystemRef<'a, 'b>,
}

impl <'a, 'b> File<'a, 'b> {
    pub(crate) fn new(first_cluster: Option<u32>, entry: Option<FileEntryInfo>, fs: FileSystemRef<'a, 'b>) -> Self {
        File {
            first_cluster, entry, fs,
            current_cluster: first_cluster,
            offset: 0,
            entry_dirty: false,
        }
    }
    
    fn update_size(&mut self) {
        match self.entry {
            Some(ref mut e) => {
                if self.offset > e.data.size() {
                    e.data.size = self.offset;
                    self.entry_dirty = true;
                }
            },
            _ => {},
        }
    }
    
    pub fn truncate(&mut self) {
        // FIXME: free clusters?
        match self.entry {
            Some(ref mut e) => {
                if e.data.size != self.offset {
                    e.data.size = self.offset;
                    self.entry_dirty = true;
                }
            },
            _ => {},
        }
    }
    
    pub(crate) fn global_pos(&self) -> Option<u64> {
        match self.current_cluster {
            Some(n) => {
                let cluster_size = self.fs.get_cluster_size();
                let offset_in_cluster = self.offset % cluster_size;
                let offset_in_fs = self.fs.offset_from_cluster(n) + (offset_in_cluster as u64);
                Some(offset_in_fs)
            },
            None => None,
        }
    }
    
    pub(crate) fn flush_dir_entry(&self) -> io::Result<()> {
        if self.entry_dirty {
            self.entry.iter().next().unwrap().write(self.fs)
        } else {
            Ok(())
        }
    }
    
    pub fn set_modified(&mut self, date_time: DateTime) {
        match self.entry {
            Some(ref mut e) => {
                e.data.set_modified(date_time);
                self.entry_dirty = true;
            },
            _ => {},
        }
    }
    
    fn bytes_left_in_file(&self) -> Option<usize> {
        match self.entry {
            Some(ref e) => {
                if e.data.is_file() {
                    Some((e.data.size - self.offset) as usize)
                } else {
                    None
                }
            },
            None => None,
        }
    }
}

impl<'a, 'b> Drop for File<'a, 'b> {
    fn drop(&mut self) {
        self.flush().expect("flush failed");
    }
}

impl<'a, 'b> Read for File<'a, 'b> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut buf_offset: usize = 0;
        let cluster_size = self.fs.get_cluster_size();
        loop {
            let current_cluster = match self.current_cluster {
                Some(n) => n,
                None => break,
            };
            let offset_in_cluster = self.offset % cluster_size;
            let bytes_left_in_cluster = (cluster_size - offset_in_cluster) as usize;
            let bytes_left_in_file = self.bytes_left_in_file().unwrap_or(bytes_left_in_cluster);
            let bytes_left_in_buf = buf.len() - buf_offset;
            let read_size = cmp::min(cmp::min(bytes_left_in_buf, bytes_left_in_cluster), bytes_left_in_file);
            if read_size == 0 {
                break;
            }
            let offset_in_fs = self.fs.offset_from_cluster(current_cluster) + (offset_in_cluster as u64);
            let read_bytes = {
                let mut disk = self.fs.disk.borrow_mut();
                disk.seek(SeekFrom::Start(offset_in_fs))?;
                disk.read(&mut buf[buf_offset..buf_offset+read_size])?
            };
            if read_bytes == 0 {
                break;
            }
            self.offset += read_bytes as u32;
            buf_offset += read_bytes;
            if self.offset % cluster_size == 0 {
                let r = self.fs.cluster_iter(current_cluster).skip(1).next();
                self.current_cluster = match r {
                    Some(Err(err)) => return Err(err),
                    Some(Ok(n)) => Some(n),
                    None => None,
                };
            }
        }
        Ok(buf_offset)
    }
}

impl<'a, 'b> Write for File<'a, 'b> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut buf_offset: usize = 0;
        let cluster_size = self.fs.get_cluster_size();
        loop {
            let current_cluster = match self.current_cluster {
                Some(n) => n,
                None => unimplemented!(), // FIXME: allocate cluster
            };
            let offset_in_cluster = self.offset % cluster_size;
            let bytes_left_in_cluster = (cluster_size - offset_in_cluster) as usize;
            let bytes_left_in_buf = buf.len() - buf_offset;
            let write_size = cmp::min(bytes_left_in_buf, bytes_left_in_cluster);
            if write_size == 0 {
                break;
            }
            let offset_in_fs = self.fs.offset_from_cluster(current_cluster) + (offset_in_cluster as u64);
            let written_bytes = {
                let mut disk = self.fs.disk.borrow_mut();
                disk.seek(SeekFrom::Start(offset_in_fs))?;
                disk.write(&buf[buf_offset..buf_offset+write_size])?
            };
            if written_bytes == 0 {
                break;
            }
            self.offset += written_bytes as u32;
            buf_offset += written_bytes;
            if self.offset % cluster_size == 0 {
                let r = self.fs.cluster_iter(current_cluster).skip(1).next();
                self.current_cluster = match r {
                    Some(Err(err)) => return Err(err),
                    Some(Ok(n)) => Some(n),
                    None => None,
                };
            }
        }
        self.update_size();
        Ok(buf_offset)
    }
    
    fn flush(&mut self) -> io::Result<()> {
        self.flush_dir_entry()?;
        let mut disk = self.fs.disk.borrow_mut();
        disk.flush()
    }
}

impl<'a, 'b> Seek for File<'a, 'b> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_offset = match pos {
            SeekFrom::Current(x) => self.offset as i64 + x,
            SeekFrom::Start(x) => x as i64,
            SeekFrom::End(x) => self.entry.iter().next().expect("cannot seek from end if size is unknown").data.size() as i64 + x,
        };
        if new_offset < 0 {
            return Err(io::Error::new(ErrorKind::InvalidInput, "invalid seek"));
        }
        let cluster_size = self.fs.get_cluster_size();
        let cluster_count = (new_offset / cluster_size as i64) as usize;
        let new_cluster = if cluster_count > 0 {
            match self.first_cluster {
                Some(n) => {
                    match self.fs.cluster_iter(n).skip(cluster_count).next() {
                        Some(Err(err)) => return Err(err),
                        Some(Ok(n)) => Some(n),
                        None => None,
                    }
                },
                None => None,
            }
        } else {
            self.first_cluster
        };
        self.offset = new_offset as u32;
        self.current_cluster = new_cluster;
        Ok(self.offset as u64)
    }
}
