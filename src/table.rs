use std::io::prelude::*;
use std::io;
use byteorder::{LittleEndian, ReadBytesExt};
use fs::FatType;
use core::iter;

pub(crate) struct FatTableData<T> {
    table: Box<[T]>,
}

impl <T> FatTableData<T> {
    pub fn new(data: Box<[T]>) -> Self {
        Self {
            table: data,
        }
    }
}

pub(crate) type FatTable12 = FatTableData<u8>;
pub(crate) type FatTable16 = FatTableData<u16>;
pub(crate) type FatTable32 = FatTableData<u32>;

pub(crate) enum FatTable {
    Fat12(FatTable12),
    Fat16(FatTable16),
    Fat32(FatTable32),
}

impl FatTable {
    pub fn from_read(rdr: &mut Read, fat_type: FatType, size: usize) -> io::Result<FatTable> {
        let table = match fat_type {
            FatType::Fat12 => FatTable::Fat12(FatTable12::from_read(rdr, size)?),
            FatType::Fat16 => FatTable::Fat16(FatTable16::from_read(rdr, size)?),
            FatType::Fat32 => FatTable::Fat32(FatTable32::from_read(rdr, size)?),
        };
        Ok(table)
    }
    
    pub fn cluster_iter(&self, cluster: u32) -> iter::Chain<iter::Once<u32>, FatClusterIterator> {
        let iter = FatClusterIterator {
            table: self,
            cluster: Some(cluster),
        };
        iter::once(cluster).chain(iter)
    }
}

trait FatNextCluster {
    fn get_next_cluster(&self, cluster: u32) -> Option<u32>;
}

impl FatNextCluster for FatTable {
    fn get_next_cluster(&self, cluster: u32) -> Option<u32> {
        match *self {
            FatTable::Fat12(ref fat) => fat.get_next_cluster(cluster),
            FatTable::Fat16(ref fat) => fat.get_next_cluster(cluster),
            FatTable::Fat32(ref fat) => fat.get_next_cluster(cluster),
        }
    }
}

impl FatTable12 {
    pub fn from_read(rdr: &mut Read, size: usize) -> io::Result<Self> {
        let mut fat = vec![0;size];
        rdr.read_exact(fat.as_mut())?;
        Ok(Self::new(fat.into_boxed_slice()))
    }
}

impl FatTable16 {
    pub fn from_read(rdr: &mut Read, size: usize) -> io::Result<Self> {
        let mut fat = vec![0;size/2];
        rdr.read_u16_into::<LittleEndian>(fat.as_mut())?;
        Ok(Self::new(fat.into_boxed_slice()))
    }
}

impl FatTable32 {
    pub fn from_read(rdr: &mut Read, size: usize) -> io::Result<Self> {
        let mut fat = vec![0;size/4];
        rdr.read_u32_into::<LittleEndian>(fat.as_mut())?;
        Ok(Self::new(fat.into_boxed_slice()))
    }
}

impl FatNextCluster for FatTable12 {
    fn get_next_cluster(&self, cluster: u32) -> Option<u32> {
        let fat_offset = cluster + (cluster / 2);
        let val1 = self.table[fat_offset as usize] as u16;
        let val2 = self.table[(fat_offset + 1) as usize] as u16;
        
        let val = if cluster & 1 == 1 {
            (val1 >> 4) | (val2 << 4)
        } else {
            val1 | (val2 & 0x0F)
        };
        if val <= 1 || val >= 0xFF7 {
            None
        } else {
            Some(val as u32)
        }
    }
}

impl FatNextCluster for FatTable16 {
    fn get_next_cluster(&self, cluster: u32) -> Option<u32> {
        let val = self.table[cluster as usize];
        if val <= 1 || val >= 0xFFF7 {
            None
        } else {
            Some(val as u32)
        }
    }
}

impl FatNextCluster for FatTable32 {
    fn get_next_cluster(&self, cluster: u32) -> Option<u32> {
        let val = self.table[cluster as usize] & 0x0FFFFFFF;
        if val <= 1 || val >= 0x0FFFFFF7 {
            None
        } else {
            Some(val)
        }
    }
}

pub(crate) struct FatClusterIterator<'a> {
    table: &'a FatTable,
    cluster: Option<u32>,
}

impl <'a> Iterator for FatClusterIterator<'a> {
    type Item = u32;

    fn next(&mut self) -> Option<Self::Item> {
        self.cluster = self.table.get_next_cluster(self.cluster.unwrap());
        self.cluster
    }
}
