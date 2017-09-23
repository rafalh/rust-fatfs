use std::io::prelude::*;
use std::io;
use byteorder::{LittleEndian, ReadBytesExt};

pub(crate) struct FatTableData<T> {
    table: Box<[T]>,
}

pub(crate) type FatTable32 = FatTableData<u32>;
pub(crate) type FatTable16 = FatTableData<u16>;
pub(crate) type FatTable12 = FatTableData<u8>;

impl <T> FatTableData<T> {
    pub fn new(data: Box<[T]>) -> FatTableData<T> {
        FatTableData::<T> {
            table: data,
        }
    }
}

impl FatTable12 {
    pub fn read(rdr: &mut Read, size: usize) -> io::Result<Self> {
        let mut fat = vec![0;size as usize];
        rdr.read_exact(fat.as_mut())?;
        Ok(FatTable12::new(fat.into_boxed_slice()))
    }
}

impl FatTable16 {
    pub fn read(rdr: &mut Read, size: usize) -> io::Result<Self> {
        let mut fat = Vec::with_capacity(size/2);
        for _ in 0..size/2 {
            fat.push(rdr.read_u16::<LittleEndian>()?);
        }
        Ok(FatTable16::new(fat.into_boxed_slice()))
    }
}

impl FatTable32 {
    pub fn read(rdr: &mut Read, size: usize) -> io::Result<Self> {
        let mut fat = Vec::with_capacity(size/4);
        for _ in 0..size/4 {
            fat.push(rdr.read_u32::<LittleEndian>()?);
        }
        Ok(FatTable32::new(fat.into_boxed_slice()))
    }
}

pub trait FatTable {
    fn get_next_cluster(&self, cluster: u32) -> Option<u32>;
}

impl FatTable for FatTable32 {
    fn get_next_cluster(&self, cluster: u32) -> Option<u32> {
        let val = self.table[cluster as usize] & 0x0FFFFFFF;
        if val <= 1 || val >= 0x0FFFFFF7 {
            None
        } else {
            Some(val)
        }
    }
}

impl FatTable for FatTable16 {
    fn get_next_cluster(&self, cluster: u32) -> Option<u32> {
        let val = self.table[cluster as usize];
        if val <= 1 || val >= 0xFFF7 {
            None
        } else {
            Some(val as u32)
        }
    }
}

impl FatTable for FatTable12 {
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
