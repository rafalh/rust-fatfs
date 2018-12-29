use byteorder::LittleEndian;
use byteorder_ext::{ReadBytesExt, WriteBytesExt};
use core::cmp;
use io;

use fs::{FatType, FsStatusFlags, ReadSeek, ReadWriteSeek};

struct Fat<T> {
    #[allow(dead_code)]
    dummy: [T; 0],
}

type Fat12 = Fat<u8>;
type Fat16 = Fat<u16>;
type Fat32 = Fat<u32>;

pub const RESERVED_FAT_ENTRIES: u32 = 2;

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
enum FatValue {
    Free,
    Data(u32),
    Bad,
    EndOfChain,
}

trait FatTrait {
    fn get_raw<T: ReadSeek>(fat: &mut T, cluster: u32) -> io::Result<u32>;
    fn get<T: ReadSeek>(fat: &mut T, cluster: u32) -> io::Result<FatValue>;
    fn set_raw<T: ReadWriteSeek>(fat: &mut T, cluster: u32, raw_value: u32) -> io::Result<()>;
    fn set<T: ReadWriteSeek>(fat: &mut T, cluster: u32, value: FatValue) -> io::Result<()>;
    fn find_free<T: ReadSeek>(fat: &mut T, start_cluster: u32, end_cluster: u32) -> io::Result<u32>;
    fn count_free<T: ReadSeek>(fat: &mut T, end_cluster: u32) -> io::Result<u32>;
}

fn read_fat<T: ReadSeek>(fat: &mut T, fat_type: FatType, cluster: u32) -> io::Result<FatValue> {
    match fat_type {
        FatType::Fat12 => Fat12::get(fat, cluster),
        FatType::Fat16 => Fat16::get(fat, cluster),
        FatType::Fat32 => Fat32::get(fat, cluster),
    }
}

fn write_fat<T: ReadWriteSeek>(fat: &mut T, fat_type: FatType, cluster: u32, value: FatValue) -> io::Result<()> {
    trace!("write FAT - cluster {} value {:?}", cluster, value);
    match fat_type {
        FatType::Fat12 => Fat12::set(fat, cluster, value),
        FatType::Fat16 => Fat16::set(fat, cluster, value),
        FatType::Fat32 => Fat32::set(fat, cluster, value),
    }
}

fn get_next_cluster<T: ReadSeek>(fat: &mut T, fat_type: FatType, cluster: u32) -> io::Result<Option<u32>> {
    let val = read_fat(fat, fat_type, cluster)?;
    match val {
        FatValue::Data(n) => Ok(Some(n)),
        _ => Ok(None),
    }
}

fn find_free_cluster<T: ReadSeek>(
    fat: &mut T,
    fat_type: FatType,
    start_cluster: u32,
    end_cluster: u32,
) -> io::Result<u32> {
    match fat_type {
        FatType::Fat12 => Fat12::find_free(fat, start_cluster, end_cluster),
        FatType::Fat16 => Fat16::find_free(fat, start_cluster, end_cluster),
        FatType::Fat32 => Fat32::find_free(fat, start_cluster, end_cluster),
    }
}

pub(crate) fn alloc_cluster<T: ReadWriteSeek>(
    fat: &mut T,
    fat_type: FatType,
    prev_cluster: Option<u32>,
    hint: Option<u32>,
    total_clusters: u32,
) -> io::Result<u32> {
    let end_cluster = total_clusters + RESERVED_FAT_ENTRIES;
    let start_cluster = match hint {
        Some(n) if n < end_cluster => n,
        _ => RESERVED_FAT_ENTRIES,
    };
    let new_cluster = match find_free_cluster(fat, fat_type, start_cluster, end_cluster) {
        Ok(n) => n,
        Err(_) if start_cluster > RESERVED_FAT_ENTRIES => {
            find_free_cluster(fat, fat_type, RESERVED_FAT_ENTRIES, start_cluster)?
        },
        Err(e) => return Err(e),
    };
    write_fat(fat, fat_type, new_cluster, FatValue::EndOfChain)?;
    if let Some(n) = prev_cluster {
        write_fat(fat, fat_type, n, FatValue::Data(new_cluster))?;
    }
    trace!("allocated cluster {}", new_cluster);
    Ok(new_cluster)
}

pub(crate) fn read_fat_flags<T: ReadSeek>(fat: &mut T, fat_type: FatType) -> io::Result<FsStatusFlags> {
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
    Ok(FsStatusFlags { dirty, io_error })
}

pub(crate) fn count_free_clusters<T: ReadSeek>(fat: &mut T, fat_type: FatType, total_clusters: u32) -> io::Result<u32> {
    let end_cluster = total_clusters + RESERVED_FAT_ENTRIES;
    match fat_type {
        FatType::Fat12 => Fat12::count_free(fat, end_cluster),
        FatType::Fat16 => Fat16::count_free(fat, end_cluster),
        FatType::Fat32 => Fat32::count_free(fat, end_cluster),
    }
}

pub(crate) fn format_fat<T: ReadWriteSeek>(
    fat: &mut T,
    fat_type: FatType,
    media: u8,
    bytes_per_fat: u64,
    total_clusters: u32,
) -> io::Result<()> {
    // init first two reserved entries to FAT ID
    match fat_type {
        FatType::Fat12 => {
            fat.write_u8(media)?;
            fat.write_u16::<LittleEndian>(0xFFFF)?;
        },
        FatType::Fat16 => {
            fat.write_u16::<LittleEndian>(media as u16 | 0xFF00)?;
            fat.write_u16::<LittleEndian>(0xFFFF)?;
        },
        FatType::Fat32 => {
            fat.write_u32::<LittleEndian>(media as u32 | 0xFFFFF00)?;
            fat.write_u32::<LittleEndian>(0xFFFFFFFF)?;
        },
    };
    // mark entries at the end of FAT as used (after FAT but before sector end)
    const BITS_PER_BYTE: u64 = 8;
    let start_cluster = total_clusters + RESERVED_FAT_ENTRIES;
    let end_cluster = (bytes_per_fat * BITS_PER_BYTE / fat_type.bits_per_fat_entry() as u64) as u32;
    for cluster in start_cluster..end_cluster {
        write_fat(fat, fat_type, cluster, FatValue::EndOfChain)?;
    }
    // mark special entries 0x0FFFFFF0 - 0x0FFFFFFF as BAD if they exists on FAT32 volume
    if end_cluster > 0x0FFFFFF0 {
        let end_bad_cluster = cmp::min(0x0FFFFFFF + 1, end_cluster);
        for cluster in 0x0FFFFFF0..end_bad_cluster {
            write_fat(fat, fat_type, cluster, FatValue::Bad)?;
        }
    }
    Ok(())
}

impl FatTrait for Fat12 {
    fn get_raw<T: ReadSeek>(fat: &mut T, cluster: u32) -> io::Result<u32> {
        let fat_offset = cluster + (cluster / 2);
        fat.seek(io::SeekFrom::Start(fat_offset as u64))?;
        let packed_val = fat.read_u16::<LittleEndian>()?;
        Ok(match cluster & 1 {
            0 => packed_val & 0x0FFF,
            _ => packed_val >> 4,
        } as u32)
    }

    fn get<T: ReadSeek>(fat: &mut T, cluster: u32) -> io::Result<FatValue> {
        let val = Self::get_raw(fat, cluster)?;
        Ok(match val {
            0 => FatValue::Free,
            0xFF7 => FatValue::Bad,
            0xFF8...0xFFF => FatValue::EndOfChain,
            n => FatValue::Data(n as u32),
        })
    }

    fn set<T: ReadWriteSeek>(fat: &mut T, cluster: u32, value: FatValue) -> io::Result<()> {
        let raw_val = match value {
            FatValue::Free => 0,
            FatValue::Bad => 0xFF7,
            FatValue::EndOfChain => 0xFFF,
            FatValue::Data(n) => n as u16,
        };
        Self::set_raw(fat, cluster, raw_val as u32)
    }

    fn set_raw<T: ReadWriteSeek>(fat: &mut T, cluster: u32, raw_val: u32) -> io::Result<()> {
        let fat_offset = cluster + (cluster / 2);
        fat.seek(io::SeekFrom::Start(fat_offset as u64))?;
        let old_packed = fat.read_u16::<LittleEndian>()?;
        fat.seek(io::SeekFrom::Start(fat_offset as u64))?;
        let new_packed = match cluster & 1 {
            0 => (old_packed & 0xF000) | raw_val as u16,
            _ => (old_packed & 0x000F) | ((raw_val as u16) << 4),
        };
        fat.write_u16::<LittleEndian>(new_packed)?;
        Ok(())
    }

    fn find_free<T: ReadSeek>(fat: &mut T, start_cluster: u32, end_cluster: u32) -> io::Result<u32> {
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
                return Err(io::Error::new(io::ErrorKind::Other, "No space left on device"));
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

    fn count_free<T: ReadSeek>(fat: &mut T, end_cluster: u32) -> io::Result<u32> {
        let mut count = 0;
        let mut cluster = RESERVED_FAT_ENTRIES;
        fat.seek(io::SeekFrom::Start((cluster * 3 / 2) as u64))?;
        let mut prev_packed_val = 0u16;
        while cluster < end_cluster {
            let res = match cluster & 1 {
                0 => fat.read_u16::<LittleEndian>(),
                _ => fat.read_u8().map(|n| n as u16),
            };
            let packed_val = match res {
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
    fn get_raw<T: ReadSeek>(fat: &mut T, cluster: u32) -> io::Result<u32> {
        fat.seek(io::SeekFrom::Start((cluster * 2) as u64))?;
        Ok(fat.read_u16::<LittleEndian>()? as u32)
    }

    fn get<T: ReadSeek>(fat: &mut T, cluster: u32) -> io::Result<FatValue> {
        let val = Self::get_raw(fat, cluster)?;
        Ok(match val {
            0 => FatValue::Free,
            0xFFF7 => FatValue::Bad,
            0xFFF8...0xFFFF => FatValue::EndOfChain,
            n => FatValue::Data(n as u32),
        })
    }

    fn set_raw<T: ReadWriteSeek>(fat: &mut T, cluster: u32, raw_value: u32) -> io::Result<()> {
        fat.seek(io::SeekFrom::Start((cluster * 2) as u64))?;
        fat.write_u16::<LittleEndian>(raw_value as u16)?;
        Ok(())
    }

    fn set<T: ReadWriteSeek>(fat: &mut T, cluster: u32, value: FatValue) -> io::Result<()> {
        let raw_value = match value {
            FatValue::Free => 0,
            FatValue::Bad => 0xFFF7,
            FatValue::EndOfChain => 0xFFFF,
            FatValue::Data(n) => n as u16,
        };
        Self::set_raw(fat, cluster, raw_value as u32)
    }

    fn find_free<T: ReadSeek>(fat: &mut T, start_cluster: u32, end_cluster: u32) -> io::Result<u32> {
        let mut cluster = start_cluster;
        fat.seek(io::SeekFrom::Start((cluster * 2) as u64))?;
        while cluster < end_cluster {
            let val = fat.read_u16::<LittleEndian>()?;
            if val == 0 {
                return Ok(cluster);
            }
            cluster += 1;
        }
        Err(io::Error::new(io::ErrorKind::Other, "No space left on device"))
    }

    fn count_free<T: ReadSeek>(fat: &mut T, end_cluster: u32) -> io::Result<u32> {
        let mut count = 0;
        let mut cluster = RESERVED_FAT_ENTRIES;
        fat.seek(io::SeekFrom::Start((cluster * 2) as u64))?;
        while cluster < end_cluster {
            let val = fat.read_u16::<LittleEndian>()?;
            if val == 0 {
                count += 1;
            }
            cluster += 1;
        }
        Ok(count)
    }
}

impl FatTrait for Fat32 {
    fn get_raw<T: ReadSeek>(fat: &mut T, cluster: u32) -> io::Result<u32> {
        fat.seek(io::SeekFrom::Start((cluster * 4) as u64))?;
        Ok(fat.read_u32::<LittleEndian>()?)
    }

    fn get<T: ReadSeek>(fat: &mut T, cluster: u32) -> io::Result<FatValue> {
        let val = Self::get_raw(fat, cluster)? & 0x0FFFFFFF;
        Ok(match val {
            0 if cluster >= 0x0FFFFFF7 && cluster <= 0x0FFFFFFF => {
                let tmp = if cluster == 0x0FFFFFF7 { "BAD_CLUSTER" } else { "end-of-chain" };
                warn!(
                    "cluster number {} is a special value in FAT to indicate {}; it should never be seen as free",
                    cluster, tmp
                );
                FatValue::Bad // avoid accidental use or allocation into a FAT chain
            },
            0 => FatValue::Free,
            0x0FFFFFF7 => FatValue::Bad,
            0x0FFFFFF8...0x0FFFFFFF => FatValue::EndOfChain,
            n if cluster >= 0x0FFFFFF7 && cluster <= 0x0FFFFFFF => {
                let tmp = if cluster == 0x0FFFFFF7 { "BAD_CLUSTER" } else { "end-of-chain" };
                warn!("cluster number {} is a special value in FAT to indicate {}; hiding potential FAT chain value {} and instead reporting as a bad sector", cluster, tmp, n);
                FatValue::Bad // avoid accidental use or allocation into a FAT chain
            },
            n => FatValue::Data(n as u32),
        })
    }

    fn set_raw<T: ReadWriteSeek>(fat: &mut T, cluster: u32, raw_value: u32) -> io::Result<()> {
        fat.seek(io::SeekFrom::Start((cluster * 4) as u64))?;
        fat.write_u32::<LittleEndian>(raw_value)?;
        Ok(())
    }

    fn set<T: ReadWriteSeek>(fat: &mut T, cluster: u32, value: FatValue) -> io::Result<()> {
        let old_reserved_bits = Self::get_raw(fat, cluster)? & 0xF0000000;

        if value == FatValue::Free && cluster >= 0x0FFFFFF7 && cluster <= 0x0FFFFFFF {
            // NOTE: it is technically allowed for them to store FAT chain loops,
            //       or even have them all store value '4' as their next cluster.
            //       Some believe only FatValue::Bad should be allowed for this edge case.
            let tmp = if cluster == 0x0FFFFFF7 { "BAD_CLUSTER" } else { "end-of-chain" };
            panic!(
                "cluster number {} is a special value in FAT to indicate {}; it should never be set as free",
                cluster, tmp
            );
        };
        let raw_val = match value {
            FatValue::Free => 0,
            FatValue::Bad => 0x0FFFFFF7,
            FatValue::EndOfChain => 0x0FFFFFFF,
            FatValue::Data(n) => n,
        };
        let raw_val = raw_val | old_reserved_bits; // must preserve original reserved values
        Self::set_raw(fat, cluster, raw_val)
    }

    fn find_free<T: ReadSeek>(fat: &mut T, start_cluster: u32, end_cluster: u32) -> io::Result<u32> {
        let mut cluster = start_cluster;
        fat.seek(io::SeekFrom::Start((cluster * 4) as u64))?;
        while cluster < end_cluster {
            let val = fat.read_u32::<LittleEndian>()? & 0x0FFFFFFF;
            if val == 0 {
                return Ok(cluster);
            }
            cluster += 1;
        }
        Err(io::Error::new(io::ErrorKind::Other, "No space left on device"))
    }

    fn count_free<T: ReadSeek>(fat: &mut T, end_cluster: u32) -> io::Result<u32> {
        let mut count = 0;
        let mut cluster = RESERVED_FAT_ENTRIES;
        fat.seek(io::SeekFrom::Start((cluster * 4) as u64))?;
        while cluster < end_cluster {
            let val = fat.read_u32::<LittleEndian>()? & 0x0FFFFFFF;
            if val == 0 {
                count += 1;
            }
            cluster += 1;
        }
        Ok(count)
    }
}

pub(crate) struct ClusterIterator<T: ReadWriteSeek> {
    fat: T,
    fat_type: FatType,
    cluster: Option<u32>,
    err: bool,
}

impl<T: ReadWriteSeek> ClusterIterator<T> {
    pub(crate) fn new(fat: T, fat_type: FatType, cluster: u32) -> Self {
        ClusterIterator { fat, fat_type, cluster: Some(cluster), err: false }
    }

    pub(crate) fn truncate(&mut self) -> io::Result<u32> {
        match self.cluster {
            Some(n) => {
                // Move to the next cluster
                self.next();
                // Mark previous cluster as end of chain
                write_fat(&mut self.fat, self.fat_type, n, FatValue::EndOfChain)?;
                // Free rest of chain
                self.free()
            },
            None => Ok(0),
        }
    }

    pub(crate) fn free(&mut self) -> io::Result<u32> {
        let mut num_free = 0;
        while let Some(n) = self.cluster {
            self.next();
            write_fat(&mut self.fat, self.fat_type, n, FatValue::Free)?;
            num_free += 1;
        }
        Ok(num_free)
    }
}

impl<T: ReadWriteSeek> Iterator for ClusterIterator<T> {
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
        self.cluster.map(Ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_fat<T: ReadWriteSeek>(fat_type: FatType, mut cur: T) {
        // based on cluster maps from Wikipedia:
        // https://en.wikipedia.org/wiki/Design_of_the_FAT_file_system#Cluster_map
        assert_eq!(read_fat(&mut cur, fat_type, 1).unwrap(), FatValue::EndOfChain);
        assert_eq!(read_fat(&mut cur, fat_type, 4).unwrap(), FatValue::Data(5));
        assert_eq!(read_fat(&mut cur, fat_type, 5).unwrap(), FatValue::Data(6));
        assert_eq!(read_fat(&mut cur, fat_type, 8).unwrap(), FatValue::EndOfChain);
        assert_eq!(read_fat(&mut cur, fat_type, 9).unwrap(), FatValue::Data(0xA));
        assert_eq!(read_fat(&mut cur, fat_type, 0xA).unwrap(), FatValue::Data(0x14));
        assert_eq!(read_fat(&mut cur, fat_type, 0x12).unwrap(), FatValue::Free);
        assert_eq!(read_fat(&mut cur, fat_type, 0x17).unwrap(), FatValue::Bad);
        assert_eq!(read_fat(&mut cur, fat_type, 0x18).unwrap(), FatValue::Bad);
        assert_eq!(read_fat(&mut cur, fat_type, 0x1B).unwrap(), FatValue::Free);

        assert_eq!(find_free_cluster(&mut cur, fat_type, 2, 0x20).unwrap(), 0x12);
        assert_eq!(find_free_cluster(&mut cur, fat_type, 0x12, 0x20).unwrap(), 0x12);
        assert_eq!(find_free_cluster(&mut cur, fat_type, 0x13, 0x20).unwrap(), 0x1B);
        assert!(find_free_cluster(&mut cur, fat_type, 0x13, 0x14).is_err());

        assert_eq!(count_free_clusters(&mut cur, fat_type, 0x1E).unwrap(), 5);

        // test allocation
        assert_eq!(alloc_cluster(&mut cur, fat_type, None, Some(0x13), 0x1E).unwrap(), 0x1B);
        assert_eq!(read_fat(&mut cur, fat_type, 0x1B).unwrap(), FatValue::EndOfChain);
        assert_eq!(alloc_cluster(&mut cur, fat_type, Some(0x1B), None, 0x1E).unwrap(), 0x12);
        assert_eq!(read_fat(&mut cur, fat_type, 0x1B).unwrap(), FatValue::Data(0x12));
        assert_eq!(read_fat(&mut cur, fat_type, 0x12).unwrap(), FatValue::EndOfChain);
        assert_eq!(count_free_clusters(&mut cur, fat_type, 0x1E).unwrap(), 3);
        // test reading from iterator
        {
            let iter = ClusterIterator::new(&mut cur, fat_type, 0x9);
            assert_eq!(iter.map(|r| r.unwrap()).collect::<Vec<_>>(), vec![0xA, 0x14, 0x15, 0x16, 0x19, 0x1A]);
        }
        // test truncating a chain
        {
            let mut iter = ClusterIterator::new(&mut cur, fat_type, 0x9);
            assert_eq!(iter.nth(3).unwrap().unwrap(), 0x16);
            iter.truncate().unwrap();
        }
        assert_eq!(read_fat(&mut cur, fat_type, 0x16).unwrap(), FatValue::EndOfChain);
        assert_eq!(read_fat(&mut cur, fat_type, 0x19).unwrap(), FatValue::Free);
        assert_eq!(read_fat(&mut cur, fat_type, 0x1A).unwrap(), FatValue::Free);
        // test freeing a chain
        {
            let mut iter = ClusterIterator::new(&mut cur, fat_type, 0x9);
            iter.free().unwrap();
        }
        assert_eq!(read_fat(&mut cur, fat_type, 0x9).unwrap(), FatValue::Free);
        assert_eq!(read_fat(&mut cur, fat_type, 0xA).unwrap(), FatValue::Free);
        assert_eq!(read_fat(&mut cur, fat_type, 0x14).unwrap(), FatValue::Free);
        assert_eq!(read_fat(&mut cur, fat_type, 0x15).unwrap(), FatValue::Free);
        assert_eq!(read_fat(&mut cur, fat_type, 0x16).unwrap(), FatValue::Free);
    }

    #[test]
    fn test_fat12() {
        let fat: Vec<u8> = vec![
            0xF0, 0xFF, 0xFF, 0x03, 0x40, 0x00, 0x05, 0x60, 0x00, 0x07, 0x80, 0x00, 0xFF, 0xAF, 0x00, 0x14, 0xC0, 0x00,
            0x0D, 0xE0, 0x00, 0x0F, 0x00, 0x01, 0x11, 0xF0, 0xFF, 0x00, 0xF0, 0xFF, 0x15, 0x60, 0x01, 0x19, 0x70, 0xFF,
            0xF7, 0xAF, 0x01, 0xFF, 0x0F, 0x00, 0x00, 0x70, 0xFF, 0x00, 0x00, 0x00,
        ];
        test_fat(FatType::Fat12, io::Cursor::new(fat));
    }

    #[test]
    fn test_fat16() {
        let fat: Vec<u8> = vec![
            0xF0, 0xFF, 0xFF, 0xFF, 0x03, 0x00, 0x04, 0x00, 0x05, 0x00, 0x06, 0x00, 0x07, 0x00, 0x08, 0x00, 0xFF, 0xFF,
            0x0A, 0x00, 0x14, 0x00, 0x0C, 0x00, 0x0D, 0x00, 0x0E, 0x00, 0x0F, 0x00, 0x10, 0x00, 0x11, 0x00, 0xFF, 0xFF,
            0x00, 0x00, 0xFF, 0xFF, 0x15, 0x00, 0x16, 0x00, 0x19, 0x00, 0xF7, 0xFF, 0xF7, 0xFF, 0x1A, 0x00, 0xFF, 0xFF,
            0x00, 0x00, 0x00, 0x00, 0xF7, 0xFF, 0x00, 0x00, 0x00, 0x00,
        ];
        test_fat(FatType::Fat16, io::Cursor::new(fat));
    }

    #[test]
    fn test_fat32() {
        let fat: Vec<u8> = vec![
            0xF0, 0xFF, 0xFF, 0x0F, 0xFF, 0xFF, 0xFF, 0x0F, 0xFF, 0xFF, 0xFF, 0x0F, 0x04, 0x00, 0x00, 0x00, 0x05, 0x00,
            0x00, 0x00, 0x06, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0x0F,
            0x0A, 0x00, 0x00, 0x00, 0x14, 0x00, 0x00, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x0D, 0x00, 0x00, 0x00, 0x0E, 0x00,
            0x00, 0x00, 0x0F, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0x0F,
            0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0x0F, 0x15, 0x00, 0x00, 0x00, 0x16, 0x00, 0x00, 0x00, 0x19, 0x00,
            0x00, 0x00, 0xF7, 0xFF, 0xFF, 0x0F, 0xF7, 0xFF, 0xFF, 0x0F, 0x1A, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0x0F,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF7, 0xFF, 0xFF, 0x0F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];
        test_fat(FatType::Fat32, io::Cursor::new(fat));
    }
}
