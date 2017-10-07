use std::io;
use byteorder::{LittleEndian, ReadBytesExt};
use fs::{FatType, FatSlice, ReadSeek};
use core::iter;

fn get_next_cluster(rdr: &mut ReadSeek, fat_type: FatType, cluster: u32) -> io::Result<Option<u32>> {
    match fat_type {
        FatType::Fat12 => get_next_cluster_12(rdr, cluster),
        FatType::Fat16 => get_next_cluster_16(rdr, cluster),
        FatType::Fat32 => get_next_cluster_32(rdr, cluster),
    }
}

fn get_next_cluster_12(rdr: &mut ReadSeek, cluster: u32) -> io::Result<Option<u32>> {
    let fat_offset = cluster + (cluster / 2);
    let mut bytes = [0;2];
    rdr.seek(io::SeekFrom::Start(fat_offset as u64))?;
    rdr.read(&mut bytes)?;
    let (val1, val2) = (bytes[0] as u16, bytes[1] as u16);
    
    let val = if cluster & 1 == 1 {
        (val1 >> 4) | (val2 << 4)
    } else {
        val1 | (val2 & 0x0F)
    };
    if val <= 1 || val >= 0xFF7 {
        Ok(None)
    } else {
        Ok(Some(val as u32))
    }
}

fn get_next_cluster_16(rdr: &mut ReadSeek, cluster: u32) -> io::Result<Option<u32>> {
    rdr.seek(io::SeekFrom::Start((cluster*2) as u64))?;
    let val = rdr.read_u16::<LittleEndian>()?;
    if val <= 1 || val >= 0xFFF7 {
        Ok(None)
    } else {
        Ok(Some(val as u32))
    }
}

fn get_next_cluster_32(rdr: &mut ReadSeek, cluster: u32) -> io::Result<Option<u32>> {
    rdr.seek(io::SeekFrom::Start((cluster*4) as u64))?;
    let val = rdr.read_u32::<LittleEndian>()? & 0x0FFFFFFF;
    if val <= 1 || val >= 0x0FFFFFF7 {
        Ok(None)
    } else {
        Ok(Some(val))
    }
}

pub(crate) struct FatClusterIterator<'a, 'b: 'a> {
    rdr: FatSlice<'a, 'b>,
    fat_type: FatType,
    cluster: Option<u32>,
    err: bool,
}

impl <'a, 'b> FatClusterIterator<'a, 'b> {
    pub(crate) fn new(rdr: FatSlice<'a, 'b>, fat_type: FatType, cluster: u32)
    -> iter::Chain<iter::Once<io::Result<u32>>, FatClusterIterator<'a, 'b>> {
        let iter = FatClusterIterator {
            rdr: rdr,
            fat_type: fat_type,
            cluster: Some(cluster),
            err: false,
        };
        iter::once(Ok(cluster)).chain(iter)
    }
}

impl <'a, 'b> Iterator for FatClusterIterator<'a, 'b> {
    type Item = io::Result<u32>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.err {
            return None;
        }
        match self.cluster {
            Some(current_cluster) => {
                self.cluster = match get_next_cluster(&mut self.rdr, self.fat_type, current_cluster) {
                    Ok(next_cluster) => next_cluster,
                    Err(err) => {
                        self.err = true;
                        return Some(Err(err));
                    },
                }
            },
            None => {},
        };
        match self.cluster {
            Some(n) => Some(Ok(n)),
            None => None,
        }
    }
}
