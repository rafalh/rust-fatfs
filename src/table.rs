use std::io;
use std::io::prelude::*;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use fs::{FatType, DiskSlice, ReadSeek};

struct Fat<T> {
    #[allow(dead_code)]
    dummy: [T; 0],
}

type Fat12 = Fat<u8>;
type Fat16 = Fat<u16>;
type Fat32 = Fat<u32>;

#[derive(Debug, Clone, Copy)]
enum FatValue {
    Free,
    Data(u32),
    Bad,
    EndOfChain,
}

trait FatTrait {
    fn get(fat: &mut ReadSeek, cluster: u32) -> io::Result<FatValue>;
    fn set(fat: &mut DiskSlice, cluster: u32, value: FatValue) -> io::Result<()>;
    fn find_free(fat: &mut ReadSeek, hint_cluster: u32) -> io::Result<u32>;
}

fn read_fat(fat: &mut ReadSeek, fat_type: FatType, cluster: u32) -> io::Result<FatValue> {
    match fat_type {
        FatType::Fat12 => Fat12::get(fat, cluster),
        FatType::Fat16 => Fat16::get(fat, cluster),
        FatType::Fat32 => Fat32::get(fat, cluster),
    }
}

fn write_fat(fat: &mut DiskSlice, fat_type: FatType, cluster: u32, value: FatValue) -> io::Result<()> {
    trace!("write FAT - cluster {} value {:?}", cluster, value);
    match fat_type {
        FatType::Fat12 => Fat12::set(fat, cluster, value),
        FatType::Fat16 => Fat16::set(fat, cluster, value),
        FatType::Fat32 => Fat32::set(fat, cluster, value),
    }
}

fn get_next_cluster(fat: &mut ReadSeek, fat_type: FatType, cluster: u32) -> io::Result<Option<u32>> {
    let val = read_fat(fat, fat_type, cluster)?;
    match val {
        FatValue::Data(n) => Ok(Some(n)),
        _ => Ok(None),
    }
}

fn find_free_cluster(fat: &mut ReadSeek, fat_type: FatType, cluster: u32) -> io::Result<u32> {
    match fat_type {
        FatType::Fat12 => Fat12::find_free(fat, cluster),
        FatType::Fat16 => Fat16::find_free(fat, cluster),
        FatType::Fat32 => Fat32::find_free(fat, cluster),
    }
}

pub(crate) fn alloc_cluster(fat: &mut DiskSlice, fat_type: FatType, prev_cluster: Option<u32>) -> io::Result<u32> {
    let new_cluster = find_free_cluster(fat, fat_type, 2)?;
    write_fat(fat, fat_type, new_cluster, FatValue::EndOfChain)?;
    if prev_cluster.is_some() {
        write_fat(fat, fat_type, prev_cluster.unwrap(), FatValue::Data(new_cluster))?; // safe
    }
    trace!("allocated cluster {}", new_cluster);
    Ok(new_cluster)
}

impl FatTrait for Fat12 {
    fn get(fat: &mut ReadSeek, cluster: u32) -> io::Result<FatValue> {
        let fat_offset = cluster + (cluster / 2);
        fat.seek(io::SeekFrom::Start(fat_offset as u64))?;
        let packed_val = fat.read_u16::<LittleEndian>()?;
        let val = match cluster & 1 {
            0 => packed_val & 0x0FFF,
            _ => packed_val >> 4,
        };
        Ok(match val {
            0 => FatValue::Free,
            0xFF7 => FatValue::Bad,
            0xFF8...0xFFF => FatValue::EndOfChain,
            n => FatValue::Data(n as u32),
        })
    }
    
    fn set(fat: &mut DiskSlice, cluster: u32, value: FatValue) -> io::Result<()> {
        let raw_val = match value {
            FatValue::Free => 0,
            FatValue::Bad => 0xFF7,
            FatValue::EndOfChain => 0xFFF,
            FatValue::Data(n) => n as u16,
        };
        let fat_offset = cluster + (cluster / 2);
        fat.seek(io::SeekFrom::Start(fat_offset as u64))?;
        let old_packed = fat.read_u16::<LittleEndian>()?;
        fat.seek(io::SeekFrom::Start(fat_offset as u64))?;
        let new_packed = match cluster & 1 {
            0 => (old_packed & 0xF000) | raw_val,
            _ => (old_packed & 0x000F) | (raw_val << 4),
        };
        fat.write_u16::<LittleEndian>(new_packed)?;
        Ok(())
    }

    fn find_free(fat: &mut ReadSeek, hint_cluster: u32) -> io::Result<u32> {
        let mut cluster = hint_cluster;
        let fat_offset = cluster + (cluster / 2);
        fat.seek(io::SeekFrom::Start(fat_offset as u64))?;
        let mut packed_val = fat.read_u16::<LittleEndian>()?;
        loop {
            let val = match cluster & 1 {
                0 => packed_val & 0x0FFF,
                _ => packed_val >> 4,
            };
            if val == 0 {
                return Ok(cluster);
            }
            cluster += 1;
            packed_val = match cluster & 1 {
                0 => fat.read_u16::<LittleEndian>()?,
                _ => {
                    let next_byte = fat.read_u8()? as u16;
                    (packed_val >> 8) | (next_byte << 8)
                },
            };
        }
    }
}

impl FatTrait for Fat16 {
    fn get(fat: &mut ReadSeek, cluster: u32) -> io::Result<FatValue> {
        fat.seek(io::SeekFrom::Start((cluster*2) as u64))?;
        let val = fat.read_u16::<LittleEndian>()?;
        Ok(match val {
            0 => FatValue::Free,
            0xFFF7 => FatValue::Bad,
            0xFFF8...0xFFFF => FatValue::EndOfChain,
            n => FatValue::Data(n as u32),
        })
    }
    
    fn set(fat: &mut DiskSlice, cluster: u32, value: FatValue) -> io::Result<()> {
        fat.seek(io::SeekFrom::Start((cluster*2) as u64))?;
        let raw_val = match value {
            FatValue::Free => 0,
            FatValue::Bad => 0xFFF7,
            FatValue::EndOfChain => 0xFFFF,
            FatValue::Data(n) => n as u16,
        };
        fat.write_u16::<LittleEndian>(raw_val)?;
        Ok(())
    }
    
    fn find_free(fat: &mut ReadSeek, hint_cluster: u32) -> io::Result<u32> {
        let mut cluster = hint_cluster;
        fat.seek(io::SeekFrom::Start((cluster*2) as u64))?;
        loop {
            let val = fat.read_u16::<LittleEndian>()?;
            if val == 0 {
                return Ok(cluster);
            }
            cluster += 1;
        }
    }
}

impl FatTrait for Fat32 {
    fn get(fat: &mut ReadSeek, cluster: u32) -> io::Result<FatValue> {
        fat.seek(io::SeekFrom::Start((cluster*4) as u64))?;
        let val = fat.read_u32::<LittleEndian>()? & 0x0FFFFFFF;
        Ok(match val {
            0 => FatValue::Free,
            0x0FFFFFF7 => FatValue::Bad,
            0x0FFFFFF8...0x0FFFFFFF => FatValue::EndOfChain,
            n => FatValue::Data(n as u32),
        })
    }
    
    fn set(fat: &mut DiskSlice, cluster: u32, value: FatValue) -> io::Result<()> {
        fat.seek(io::SeekFrom::Start((cluster*4) as u64))?;
        let raw_val = match value {
            FatValue::Free => 0,
            FatValue::Bad => 0x0FFFFFF7,
            FatValue::EndOfChain => 0x0FFFFFFF,
            FatValue::Data(n) => n,
        };
        fat.write_u32::<LittleEndian>(raw_val)?;
        Ok(())
    }
    
    fn find_free(fat: &mut ReadSeek, hint_cluster: u32) -> io::Result<u32> {
        let mut cluster = hint_cluster;
        fat.seek(io::SeekFrom::Start((cluster*4) as u64))?;
        loop {
            let val = fat.read_u32::<LittleEndian>()? & 0x0FFFFFFF;
            if val == 0 {
                return Ok(cluster);
            }
            cluster += 1;
        }
    }
}

pub(crate) struct ClusterIterator<'a, 'b: 'a> {
    fat: DiskSlice<'a, 'b>,
    fat_type: FatType,
    cluster: Option<u32>,
    err: bool,
}

impl <'a, 'b> ClusterIterator<'a, 'b> {
    pub(crate) fn new(fat: DiskSlice<'a, 'b>, fat_type: FatType, cluster: u32)
    -> ClusterIterator<'a, 'b> {
        ClusterIterator {
            fat: fat,
            fat_type: fat_type,
            cluster: Some(cluster),
            err: false,
        }
    }
    
    pub(crate) fn truncate(&mut self) -> io::Result<()> {
        match self.cluster {
            Some(n) => {
                write_fat(&mut self.fat, self.fat_type, n, FatValue::EndOfChain)?;
                self.next();
                self.free()
            },
            None => Ok(()),
        }
    }
    
    pub(crate) fn free(&mut self) -> io::Result<()> {
        loop {
            let prev = self.cluster;
            self.next();
            match prev {
                Some(n) => write_fat(&mut self.fat, self.fat_type, n, FatValue::Free)?,
                None => break,
            };
        }
        Ok(())
    }
}

impl <'a, 'b> Iterator for ClusterIterator<'a, 'b> {
    type Item = io::Result<u32>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.err {
            return None;
        }
        match self.cluster {
            Some(current_cluster) => {
                self.cluster = match get_next_cluster(&mut self.fat, self.fat_type, current_cluster) {
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
