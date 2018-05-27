use io;
use io::prelude::*;
use byteorder::LittleEndian;
use byteorder_ext::{ReadBytesExt, WriteBytesExt};
use fs::{FatType, FsStatusFlags, DiskSlice, ReadSeek};

struct Fat<T> {
    #[allow(dead_code)]
    dummy: [T; 0],
}

type Fat12 = Fat<u8>;
type Fat16 = Fat<u16>;
type Fat32 = Fat<u32>;

const RESERVED_FAT_ENTRIES: u32 = 2;

#[derive(Debug, Clone, Copy)]
enum FatValue {
    Free,
    Data(u32),
    Bad,
    EndOfChain,
}

trait FatTrait {
    fn get_raw(fat: &mut ReadSeek, cluster: u32) -> io::Result<u32>;
    fn get(fat: &mut ReadSeek, cluster: u32) -> io::Result<FatValue>;
    fn set(fat: &mut DiskSlice, cluster: u32, value: FatValue) -> io::Result<()>;
    fn find_free(fat: &mut ReadSeek, start_cluster: u32, end_cluster: u32) -> io::Result<u32>;
    fn count_free(fat: &mut ReadSeek, end_cluster: u32) -> io::Result<u32>;
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

fn find_free_cluster(fat: &mut ReadSeek, fat_type: FatType, start_cluster: u32, end_cluster: u32) -> io::Result<u32> {
    match fat_type {
        FatType::Fat12 => Fat12::find_free(fat, start_cluster, end_cluster),
        FatType::Fat16 => Fat16::find_free(fat, start_cluster, end_cluster),
        FatType::Fat32 => Fat32::find_free(fat, start_cluster, end_cluster),
    }
}

pub(crate) fn alloc_cluster(fat: &mut DiskSlice, fat_type: FatType, prev_cluster: Option<u32>, hint: Option<u32>, total_clusters: u32) -> io::Result<u32> {
    let end_cluster = total_clusters + RESERVED_FAT_ENTRIES;
    let start_cluster = match hint {
        Some(n) if n < end_cluster => n,
        _ => RESERVED_FAT_ENTRIES,
    };
    let new_cluster = match find_free_cluster(fat, fat_type, start_cluster, end_cluster) {
        Ok(n) => n,
        Err(_) if start_cluster > RESERVED_FAT_ENTRIES => find_free_cluster(fat, fat_type, RESERVED_FAT_ENTRIES, start_cluster)?,
        Err(e) => return Err(e),
    };
    write_fat(fat, fat_type, new_cluster, FatValue::EndOfChain)?;
    if let Some(n) = prev_cluster {
        write_fat(fat, fat_type, n, FatValue::Data(new_cluster))?;
    }
    trace!("allocated cluster {}", new_cluster);
    Ok(new_cluster)
}

pub(crate) fn read_fat_flags(fat: &mut DiskSlice, fat_type: FatType) -> io::Result<FsStatusFlags> {
    // check MSB (except in FAT12)
    let val = match fat_type {
        FatType::Fat12 => 0xFFF,
        FatType::Fat16 => Fat16::get_raw(fat, 1)?,
        FatType::Fat32 => Fat32::get_raw(fat, 1)?,
    };
    let dirty = match fat_type {
        FatType::Fat12 => false,
        FatType::Fat16 => val & (1 << 15) == 0,
        FatType::Fat32 => val & (1 << 27) == 0,
    };
    let io_error = match fat_type {
        FatType::Fat12 => false,
        FatType::Fat16 => val & (1 << 14) == 0,
        FatType::Fat32 => val & (1 << 26) == 0,
    };
    Ok(FsStatusFlags {
        dirty, io_error,
    })
}

pub(crate) fn count_free_clusters(fat: &mut ReadSeek, fat_type: FatType, total_clusters: u32) -> io::Result<u32> {
    let end_cluster = total_clusters + RESERVED_FAT_ENTRIES;
    match fat_type {
        FatType::Fat12 => Fat12::count_free(fat, end_cluster),
        FatType::Fat16 => Fat16::count_free(fat, end_cluster),
        FatType::Fat32 => Fat32::count_free(fat, end_cluster),
    }
}

impl FatTrait for Fat12 {
    fn get_raw(fat: &mut ReadSeek, cluster: u32) -> io::Result<u32> {
        let fat_offset = cluster + (cluster / 2);
        fat.seek(io::SeekFrom::Start(fat_offset as u64))?;
        let packed_val = fat.read_u16::<LittleEndian>()?;
        Ok(match cluster & 1 {
            0 => packed_val & 0x0FFF,
            _ => packed_val >> 4,
        } as u32)
    }

    fn get(fat: &mut ReadSeek, cluster: u32) -> io::Result<FatValue> {
        let val = Self::get_raw(fat, cluster)?;
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

    fn find_free(fat: &mut ReadSeek, start_cluster: u32, end_cluster: u32) -> io::Result<u32> {
        let mut cluster = start_cluster;
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
            if cluster == end_cluster {
                return Err(io::Error::new(io::ErrorKind::Other, "end of FAT reached"));
            }
            packed_val = match cluster & 1 {
                0 => fat.read_u16::<LittleEndian>()?,
                _ => {
                    let next_byte = fat.read_u8()? as u16;
                    (packed_val >> 8) | (next_byte << 8)
                },
            };
        }
    }

    fn count_free(fat: &mut ReadSeek, end_cluster: u32) -> io::Result<u32> {
        let mut count = 0;
        let mut cluster = RESERVED_FAT_ENTRIES;
        fat.seek(io::SeekFrom::Start((cluster*3/2) as u64))?;
        let mut prev_packed_val = 0u16;
        while cluster < end_cluster {
            let res = match cluster & 1 {
                0 => fat.read_u16::<LittleEndian>(),
                _ => fat.read_u8().map(|n| n as u16),
            };
            let packed_val = match res {
                Err(ref err) if err.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(err) => return Err(err),
                Ok(n) => n,
            };
            let val = match cluster & 1 {
                0 => packed_val & 0x0FFF,
                _ => (packed_val << 8) | (prev_packed_val >> 12),
            };
            prev_packed_val = packed_val;
            if val == 0 {
                count += 1;
            }
            cluster += 1;
        }
        Ok(count)
    }
}

impl FatTrait for Fat16 {
    fn get_raw(fat: &mut ReadSeek, cluster: u32) -> io::Result<u32> {
        fat.seek(io::SeekFrom::Start((cluster*2) as u64))?;
        Ok(fat.read_u16::<LittleEndian>()? as u32)
    }

    fn get(fat: &mut ReadSeek, cluster: u32) -> io::Result<FatValue> {
        let val = Self::get_raw(fat, cluster)?;
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

    fn find_free(fat: &mut ReadSeek, start_cluster: u32, end_cluster: u32) -> io::Result<u32> {
        let mut cluster = start_cluster;
        fat.seek(io::SeekFrom::Start((cluster*2) as u64))?;
        while cluster < end_cluster {
            let val = fat.read_u16::<LittleEndian>()?;
            if val == 0 {
                return Ok(cluster);
            }
            cluster += 1;
        }
        Err(io::Error::new(io::ErrorKind::Other, "end of FAT reached"))
    }

    fn count_free(fat: &mut ReadSeek, end_cluster: u32) -> io::Result<u32> {
        let mut count = 0;
        let mut cluster = RESERVED_FAT_ENTRIES;
        fat.seek(io::SeekFrom::Start((cluster*2) as u64))?;
        while cluster < end_cluster {
            match fat.read_u16::<LittleEndian>() {
                Err(ref err) if err.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(err) => return Err(err),
                Ok(0) => count += 1,
                _ => {},
            }
            cluster += 1;
        }
        Ok(count)
    }
}

impl FatTrait for Fat32 {
    fn get_raw(fat: &mut ReadSeek, cluster: u32) -> io::Result<u32> {
        fat.seek(io::SeekFrom::Start((cluster*4) as u64))?;
        Ok(fat.read_u32::<LittleEndian>()? & 0x0FFFFFFF)
    }

    fn get(fat: &mut ReadSeek, cluster: u32) -> io::Result<FatValue> {
        let val = Self::get_raw(fat, cluster)?;
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

    fn find_free(fat: &mut ReadSeek, start_cluster: u32, end_cluster: u32) -> io::Result<u32> {
        let mut cluster = start_cluster;
        fat.seek(io::SeekFrom::Start((cluster*4) as u64))?;
        while cluster < end_cluster {
            let val = fat.read_u32::<LittleEndian>()? & 0x0FFFFFFF;
            if val == 0 {
                return Ok(cluster);
            }
            cluster += 1;
        }
        Err(io::Error::new(io::ErrorKind::Other, "end of FAT reached"))
    }

    fn count_free(fat: &mut ReadSeek, end_cluster: u32) -> io::Result<u32> {
        let mut count = 0;
        let mut cluster = RESERVED_FAT_ENTRIES;
        fat.seek(io::SeekFrom::Start((cluster*4) as u64))?;
        while cluster < end_cluster {
            match fat.read_u32::<LittleEndian>() {
                Err(ref err) if err.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(err) => return Err(err),
                Ok(0) => count += 1,
                _ => {},
            }
            cluster += 1;
        }
        Ok(count)
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
                // Move to the next cluster
                self.next();
                // Mark previous cluster as end of chain
                write_fat(&mut self.fat, self.fat_type, n, FatValue::EndOfChain)?;
                // Free rest of chain
                self.free()
            },
            None => Ok(()),
        }
    }

    pub(crate) fn free(&mut self) -> io::Result<()> {
        while let Some(n) = self.cluster {
            self.next();
            write_fat(&mut self.fat, self.fat_type, n, FatValue::Free)?;
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
        if let Some(current_cluster) = self.cluster {
            self.cluster = match get_next_cluster(&mut self.fat, self.fat_type, current_cluster) {
                Ok(next_cluster) => next_cluster,
                Err(err) => {
                    self.err = true;
                    return Some(Err(err));
                },
            }
        }
        self.cluster.map(|n| Ok(n))
    }
}
