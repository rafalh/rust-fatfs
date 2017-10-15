use std::cmp;
use std::io::prelude::*;
use std::io::{SeekFrom, ErrorKind};
use std::io;

use fs::FileSystemRef;
use dir::{FileEntryInfo, DateTime};

#[derive(Clone)]
pub struct File<'a, 'b: 'a> {
    // Note first_cluster is None if file is empty
    first_cluster: Option<u32>,
    // Note: if offset points between clusters current_cluster is the previous cluster
    current_cluster: Option<u32>,
    // current position in this file
    offset: u32,
    // file dir entry - None for root dir
    entry: Option<FileEntryInfo>,
    // should file dir entry be flushed?
    entry_dirty: bool,
    fs: FileSystemRef<'a, 'b>,
}

impl <'a, 'b> File<'a, 'b> {
    pub(crate) fn new(first_cluster: Option<u32>, entry: Option<FileEntryInfo>, fs: FileSystemRef<'a, 'b>) -> Self {
        File {
            first_cluster, entry, fs,
            current_cluster: None, // cluster before first one
            offset: 0,
            entry_dirty: false,
        }
    }
    
    fn update_size(&mut self) {
        let offset = self.offset;
        match self.entry {
            Some(ref mut e) => {
                if e.data.size().map_or(false, |s| offset > s) {
                    e.data.set_size(offset);
                    self.entry_dirty = true;
                }
            },
            _ => {},
        }
    }
    
    pub fn truncate(&mut self) -> io::Result<()> {
        let offset = self.offset;
        match self.entry {
            Some(ref mut e) => {
                if e.data.size().map_or(false, |s| offset == s) {
                    return Ok(());
                }
                
                e.data.set_size(self.offset);
                if self.offset == 0 {
                    e.data.set_first_cluster(None);
                }
                self.entry_dirty = true;
            },
            _ => {},
        }
        if self.offset > 0 {
            debug_assert!(self.current_cluster.is_some());
            self.fs.cluster_iter(self.current_cluster.unwrap()).truncate() // safe
        } else {
            debug_assert!(self.current_cluster.is_none());
            match self.first_cluster {
                Some(n) => self.fs.cluster_iter(n).free()?,
                _ => {},
            }
            self.first_cluster = None;
            Ok(())
        }
    }
    
    pub(crate) fn global_pos(&self) -> Option<u64> {
        // Returns current position relative to filesystem start
        // Note: when between clusters it returns position after previous cluster
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
            match self.entry {
                Some(ref e) => e.write(self.fs)?,
                _ => {},
            }
        }
        Ok(())
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
            Some(ref e) => e.data.size().map(|s| (s - self.offset) as usize),
            None => None,
        }
    }
    
    fn set_first_cluster(&mut self, cluster: u32) {
        self.first_cluster = Some(cluster);
        match self.entry {
            Some(ref mut e) => {
                e.data.set_first_cluster(self.first_cluster);
            },
            None => {},
        }
        self.entry_dirty = true;
    }
}

impl<'a, 'b> Drop for File<'a, 'b> {
    fn drop(&mut self) {
        self.flush().expect("flush failed");
    }
}

impl<'a, 'b> Read for File<'a, 'b> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let cluster_size = self.fs.get_cluster_size();
        let current_cluster_opt = if self.offset % cluster_size == 0 {
            // next cluster
            match self.current_cluster {
                None => self.first_cluster,
                Some(n) => {
                    let r = self.fs.cluster_iter(n).next();
                    match r {
                        Some(Err(err)) => return Err(err),
                        Some(Ok(n)) => Some(n),
                        None => None,
                    }
                },
            }
        } else {
            self.current_cluster
        };
        let current_cluster = match current_cluster_opt {
            Some(n) => n,
            None => return Ok(0),
        };
        let offset_in_cluster = self.offset % cluster_size;
        let bytes_left_in_cluster = (cluster_size - offset_in_cluster) as usize;
        let bytes_left_in_file = self.bytes_left_in_file().unwrap_or(bytes_left_in_cluster);
        let read_size = cmp::min(cmp::min(buf.len(), bytes_left_in_cluster), bytes_left_in_file);
        if read_size == 0 {
            return Ok(0);
        }
        trace!("read {} bytes in cluster {}", read_size, current_cluster);
        let offset_in_fs = self.fs.offset_from_cluster(current_cluster) + (offset_in_cluster as u64);
        let read_bytes = {
            let mut disk = self.fs.disk.borrow_mut();
            disk.seek(SeekFrom::Start(offset_in_fs))?;
            disk.read(&mut buf[..read_size])?
        };
        if read_bytes == 0 {
            return Ok(0);
        }
        self.offset += read_bytes as u32;
        self.current_cluster = Some(current_cluster);
        Ok(read_bytes)
    }
}

impl<'a, 'b> Write for File<'a, 'b> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let cluster_size = self.fs.get_cluster_size();
        let offset_in_cluster = self.offset % cluster_size;
        let bytes_left_in_cluster = (cluster_size - offset_in_cluster) as usize;
        let write_size = cmp::min(buf.len(), bytes_left_in_cluster);
        // Exit early if we are going to write no data
        if write_size == 0 {
            return Ok(0);
        }
        // Get cluster for write possibly allocating new one
        let current_cluster = if self.offset % cluster_size == 0 {
            // next cluster
            let next_cluster = match self.current_cluster {
                None => self.first_cluster,
                Some(n) => {
                    let r = self.fs.cluster_iter(n).next();
                    match r {
                        Some(Err(err)) => return Err(err),
                        Some(Ok(n)) => Some(n),
                        None => None,
                    }
                },
            };
            match next_cluster {
                Some(n) => n,
                None => {
                    // end of chain reached - allocate new cluster
                    let new_cluster = self.fs.alloc_cluster(self.current_cluster)?;
                    if self.first_cluster.is_none() {
                        self.set_first_cluster(new_cluster);
                    }
                    new_cluster
                },
            }
        } else {
            // self.current_cluster should be a valid cluster
            match self.current_cluster {
                Some(n) => n,
                None => panic!("Offset inside cluster but no cluster allocated"),
            }
        };
        trace!("write {} bytes in cluster {}", write_size, current_cluster);
        let offset_in_fs = self.fs.offset_from_cluster(current_cluster) + (offset_in_cluster as u64);
        let written_bytes = {
            let mut disk = self.fs.disk.borrow_mut();
            disk.seek(SeekFrom::Start(offset_in_fs))?;
            disk.write(&buf[..write_size])?
        };
        if written_bytes == 0 {
            return Ok(0);
        }
        self.offset += written_bytes as u32;
        self.current_cluster = Some(current_cluster);
        self.update_size();
        Ok(written_bytes)
    }
    
    fn flush(&mut self) -> io::Result<()> {
        self.flush_dir_entry()?;
        let mut disk = self.fs.disk.borrow_mut();
        disk.flush()
    }
}

impl<'a, 'b> Seek for File<'a, 'b> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let mut new_pos = match pos {
            SeekFrom::Current(x) => self.offset as i64 + x,
            SeekFrom::Start(x) => x as i64,
            SeekFrom::End(x) => self.entry.iter().next().map_or(None, |e| e.data.size()).expect("cannot seek from end if size is unknown") as i64 + x,
        };
        if new_pos < 0 {
            return Err(io::Error::new(ErrorKind::InvalidInput, "invalid seek"));
        }
        new_pos = match self.entry {
            Some(ref e) => {
                if e.data.size().map_or(false, |s| new_pos > s as i64) {
                    info!("seek beyond end of file");
                    e.data.size().unwrap() as i64 // safe
                } else {
                    new_pos
                }
            },
            _ => new_pos,
        };
        trace!("file seek {} -> {} - entry {:?}", self.offset, new_pos, self.entry);
        if new_pos == self.offset as i64 {
            return Ok(self.offset as u64);
        }
        let cluster_size = self.fs.get_cluster_size();
        let new_cluster = if new_pos == 0 {
            None
        } else {
            // get number of clusters to seek (favoring previous cluster in corner case)
            let cluster_count = ((new_pos - 1) / cluster_size as i64) as isize;
            match self.first_cluster {
                Some(n) => {
                    let mut cluster = n;
                    let mut iter = self.fs.cluster_iter(n);
                    for i in 0..cluster_count {
                        cluster = match iter.next() {
                            Some(Err(err)) => return Err(err),
                            Some(Ok(n)) => n,
                            None => {
                                // chain ends before new position - seek to end of last cluster
                                new_pos = (i + 1) as i64 * cluster_size as i64;
                                break;
                            },
                        };
                    }
                    Some(cluster)
                },
                None => {
                    // empty file - always seek to 0
                    new_pos = 0;
                    None
                },
            }
        };
        self.offset = new_pos as u32;
        self.current_cluster = new_cluster;
        Ok(self.offset as u64)
    }
}
