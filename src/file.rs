use std::io::prelude::*;
use std::io;
use fs::FatFileSystem;

#[allow(dead_code)]
pub struct FatFile {
    first_sector: u32,
    size: u32,
    offset: u32,
}

impl FatFile {
    pub fn new(first_sector: u32, size: u32) -> FatFile {
        FatFile { first_sector, size, offset: 0 }
    }
}

impl<T: Read+Seek> FatFileSystem<T> {
    
    pub fn file_from_cluster(&mut self, cluster: u32, size: u32) -> FatFile {
        FatFile {
            first_sector: self.sector_from_cluster(cluster),
            size: size,
            offset: 0,
        }
    }
    
    pub fn read(&mut self, file: &mut FatFile, buf: &mut [u8]) -> io::Result<usize> {
        self.seek_to_sector(file.first_sector as u64)?;
        let size = self.rdr.read(buf)?;
        file.offset += size as u32;
        Ok(size)
    }
}
