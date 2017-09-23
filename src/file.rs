use std::cmp;
use std::io::prelude::*;
use std::io::SeekFrom;
use std::io;

use fs::FatSharedStateRef;

#[allow(dead_code)]
pub struct FatFile {
    first_cluster: u32,
    size: Option<u32>,
    offset: u32,
    current_cluster: Option<u32>,
    state: FatSharedStateRef,
}

impl FatFile {
    pub(crate) fn new(first_cluster: u32, size: Option<u32>, state: FatSharedStateRef) -> FatFile {
        FatFile {
            first_cluster, size, state,
            current_cluster: Some(first_cluster),
            offset: 0,
        }
    }
}

impl Read for FatFile {
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
                self.current_cluster = state.table.get_next_cluster(current_cluster);
            }
        }
        Ok(buf_offset)
    }
}
