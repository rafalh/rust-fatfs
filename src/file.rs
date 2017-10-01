use std::cmp;
use std::io::prelude::*;
use std::io::{SeekFrom, ErrorKind};
use std::io;

use fs::FatSharedStateRef;


pub struct FatFile<'a, 'b: 'a> {
    first_cluster: u32,
    size: Option<u32>,
    offset: u32,
    current_cluster: Option<u32>,
    state: FatSharedStateRef<'a, 'b>,
}

impl <'a, 'b> FatFile<'a, 'b> {
    pub(crate) fn new(first_cluster: u32, size: Option<u32>, state: FatSharedStateRef<'a, 'b>) -> Self {
        FatFile {
            first_cluster, size, state,
            current_cluster: Some(first_cluster),
            offset: 0,
        }
    }
}

impl <'a, 'b> Read for FatFile<'a, 'b> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut buf_offset: usize = 0;
        let cluster_size = self.state.borrow().get_cluster_size();
        let mut state = self.state.borrow_mut();
        loop {
            let offset_in_cluster = self.offset % cluster_size;
            let bytes_left_in_cluster = (cluster_size - offset_in_cluster) as usize;
            let bytes_left_in_file = self.size.map(|size| (size - self.offset) as usize).unwrap_or(bytes_left_in_cluster);
            let bytes_left_in_buf = buf.len() - buf_offset;
            let read_size = cmp::min(cmp::min(bytes_left_in_buf, bytes_left_in_cluster), bytes_left_in_file);
            if read_size == 0 {
                break;
            }
            let current_cluster = self.current_cluster.unwrap();
            let offset_in_fs = state.offset_from_cluster(current_cluster) + (offset_in_cluster as u64);
            state.rdr.seek(SeekFrom::Start(offset_in_fs))?;
            let read_bytes = state.rdr.read(&mut buf[buf_offset..buf_offset+read_size])?;
            if read_bytes == 0 {
                break;
            }
            self.offset += read_bytes as u32;
            buf_offset += read_bytes;
            if self.offset % cluster_size == 0 {
                self.current_cluster = state.table.cluster_iter(current_cluster).skip(1).next();
            }
        }
        Ok(buf_offset)
    }
}

impl <'a, 'b> Seek for FatFile<'a, 'b> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_offset = match pos {
            SeekFrom::Current(x) => self.offset as i64 + x,
            SeekFrom::Start(x) => x as i64,
            SeekFrom::End(x) => self.size.unwrap() as i64 + x,
        };
        if new_offset < 0 || (self.size.is_some() && new_offset as u64 > self.size.unwrap() as u64) {
            return Err(io::Error::new(ErrorKind::InvalidInput, "invalid seek"));
        }
        let cluster_size = self.state.borrow().get_cluster_size();
        let cluster_count = (new_offset / cluster_size as i64) as usize;
        let mut new_cluster = Some(self.first_cluster);
        let state = self.state.borrow_mut();
        if cluster_count > 0 {
            new_cluster = state.table.cluster_iter(new_cluster.unwrap()).skip(cluster_count).next();
        }
        self.offset = new_offset as u32;
        self.current_cluster = new_cluster;
        Ok(self.offset as u64)
    }
}
