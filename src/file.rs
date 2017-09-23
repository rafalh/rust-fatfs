use std::cmp;
use std::io::prelude::*;
use std::io::SeekFrom;
use std::io;

use fs::FatSharedStateRef;

#[allow(dead_code)]
pub struct FatFile {
    first_sector: u32,
    size: u32,
    offset: u32,
    state: FatSharedStateRef,
}

impl FatFile {
    pub(crate) fn new(first_cluster: u32, size: u32, state: FatSharedStateRef) -> FatFile {
        let first_sector = state.borrow().sector_from_cluster(first_cluster);
        FatFile {
            first_sector, size, state, offset: 0,
        }
    }
}

impl Read for FatFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let (offset, read_size) = {
            let state = self.state.borrow();
            let offset = state.offset_from_sector(self.first_sector) + self.offset as u64;
            let mut read_size = cmp::min((self.size - self.offset) as usize, buf.len());
            // FIXME: allow only one cluster for now
            read_size = cmp::min(read_size, (state.get_cluster_size() - self.offset) as usize);
            (offset, read_size)
        };
        let mut state = self.state.borrow_mut();
        state.rdr.seek(SeekFrom::Start(offset))?;
        let size = state.rdr.read(&mut buf[..read_size])?;
        self.offset += size as u32;
        Ok(size)
    }
}
