use core::cell::RefCell;
use core::cmp;
use std::io::prelude::*;
use std::io::{Error, ErrorKind, SeekFrom};
use std::io;
use byteorder::{LittleEndian, ReadBytesExt};

use file::File;
use dir::{DirRawStream, Dir};
use table::{ClusterIterator, alloc_cluster};

// FAT implementation based on:
//   http://wiki.osdev.org/FAT
//   https://www.win.tue.nl/~aeb/linux/fs/fat/fat-1.html

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum FatType {
    Fat12, Fat16, Fat32,
}

pub trait ReadSeek: Read + Seek {}
impl<T> ReadSeek for T where T: Read + Seek {}

pub trait ReadWriteSeek: Read + Write + Seek {}
impl<T> ReadWriteSeek for T where T: Read + Write + Seek {}

#[allow(dead_code)]
#[derive(Default, Debug, Clone)]
pub(crate) struct BiosParameterBlock {
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    reserved_sectors: u16,
    fats: u8,
    root_entries: u16,
    total_sectors_16: u16,
    media: u8,
    sectors_per_fat_16: u16,
    sectors_per_track: u16,
    heads: u16,
    hidden_sectors: u32,
    total_sectors_32: u32,

    // Extended BIOS Parameter Block
    sectors_per_fat_32: u32,
    extended_flags: u16,
    fs_version: u16,
    root_dir_first_cluster: u32,
    fs_info_sector: u16,
    backup_boot_sector: u16,
    reserved_0: [u8; 12],
    drive_num: u8,
    reserved_1: u8,
    ext_sig: u8,
    volume_id: u32,
    volume_label: [u8; 11],
    fs_type_label: [u8; 8],
}

#[allow(dead_code)]
pub(crate) struct BootRecord {
    bootjmp: [u8; 3],
    oem_name: [u8; 8],
    bpb: BiosParameterBlock,
    boot_code: [u8; 448],
    boot_sig: [u8; 2],
}

impl Default for BootRecord {
    fn default() -> BootRecord {
        BootRecord {
            bootjmp: Default::default(),
            oem_name: Default::default(),
            bpb: Default::default(),
            boot_code: [0; 448],
            boot_sig: Default::default(),
        }
    }
}

pub(crate) type FileSystemRef<'a, 'b: 'a> = &'a FileSystem<'b>;

/// FAT filesystem main struct.
pub struct FileSystem<'a> {
    pub(crate) disk: RefCell<&'a mut ReadWriteSeek>,
    pub(crate) fat_type: FatType,
    pub(crate) boot: BootRecord,
    pub(crate) first_data_sector: u32,
    pub(crate) root_dir_sectors: u32,
}

impl <'a> FileSystem<'a> {
    /// Creates new filesystem object instance.
    ///
    /// Note: creating multiple filesystem objects with one underlying device/disk image can
    /// cause filesystem corruption.
    pub fn new<T: ReadWriteSeek>(disk: &'a mut T) -> io::Result<FileSystem<'a>> {
        let boot = Self::read_boot_record(disk)?;
        if boot.boot_sig != [0x55, 0xAA] {
            return Err(Error::new(ErrorKind::Other, "invalid signature"));
        }

        let total_sectors = if boot.bpb.total_sectors_16 == 0 { boot.bpb.total_sectors_32 } else { boot.bpb.total_sectors_16 as u32 };
        let sectors_per_fat = if boot.bpb.sectors_per_fat_16 == 0 { boot.bpb.sectors_per_fat_32 } else { boot.bpb.sectors_per_fat_16 as u32 };
        let root_dir_sectors = (((boot.bpb.root_entries * 32) + (boot.bpb.bytes_per_sector - 1)) / boot.bpb.bytes_per_sector) as u32;
        let first_data_sector = boot.bpb.reserved_sectors as u32 + (boot.bpb.fats as u32 * sectors_per_fat) + root_dir_sectors;
        let data_sectors = total_sectors - (boot.bpb.reserved_sectors as u32 + (boot.bpb.fats as u32 * sectors_per_fat) + root_dir_sectors as u32);
        let total_clusters = data_sectors / boot.bpb.sectors_per_cluster as u32;
        let fat_type = Self::fat_type_from_clusters(total_clusters);

        Ok(FileSystem {
            disk: RefCell::new(disk),
            fat_type,
            boot,
            first_data_sector,
            root_dir_sectors,
        })
    }

    /// Returns type of used File Allocation Table (FAT).
    pub fn fat_type(&self) -> FatType {
        self.fat_type
    }

    /// Returns volume identifier read from BPB in Boot Sector.
    pub fn volume_id(&self) -> u32 {
        self.boot.bpb.volume_id
    }

    /// Returns volume label from BPB in Boot Sector.
    ///
    /// Note: File with VOLUME_ID attribute in root directory is ignored by this library.
    /// Only label from BPB is used.
    pub fn volume_label(&self) -> String {
        String::from_utf8_lossy(&self.boot.bpb.volume_label).trim_right().to_string()
    }

    /// Returns root directory object allowing futher penetration of filesystem structure.
    pub fn root_dir<'b>(&'b self) -> Dir<'b, 'a> {
        let root_rdr = {
            match self.fat_type {
                FatType::Fat12 | FatType::Fat16 => DirRawStream::Root(DiskSlice::from_sectors(
                   self.first_data_sector - self.root_dir_sectors, self.root_dir_sectors, 1, self)),
                _ => DirRawStream::File(File::new(Some(self.boot.bpb.root_dir_first_cluster), None, self)),
            }
        };
        Dir::new(root_rdr, self)
    }

    fn read_bpb(rdr: &mut Read) -> io::Result<BiosParameterBlock> {
        let mut bpb: BiosParameterBlock = Default::default();
        bpb.bytes_per_sector = rdr.read_u16::<LittleEndian>()?;
        bpb.sectors_per_cluster = rdr.read_u8()?;
        bpb.reserved_sectors = rdr.read_u16::<LittleEndian>()?;
        bpb.fats = rdr.read_u8()?;
        bpb.root_entries = rdr.read_u16::<LittleEndian>()? ;
        bpb.total_sectors_16 = rdr.read_u16::<LittleEndian>()?;
        bpb.media = rdr.read_u8()?;
        bpb.sectors_per_fat_16 = rdr.read_u16::<LittleEndian>()?;
        bpb.sectors_per_track = rdr.read_u16::<LittleEndian>()?;
        bpb.heads = rdr.read_u16::<LittleEndian>()?;
        bpb.hidden_sectors = rdr.read_u32::<LittleEndian>()?;
        bpb.total_sectors_32 = rdr.read_u32::<LittleEndian>()?;

        // sanity checks
        if bpb.bytes_per_sector < 512 {
            return Err(Error::new(ErrorKind::Other, "invalid bytes_per_sector value in BPB"));
        }
        if bpb.sectors_per_cluster < 1 {
            return Err(Error::new(ErrorKind::Other, "invalid sectors_per_cluster value in BPB"));
        }
        if bpb.reserved_sectors < 1 {
            return Err(Error::new(ErrorKind::Other, "invalid reserved_sectors value in BPB"));
        }
        if bpb.fats == 0 {
            return Err(Error::new(ErrorKind::Other, "invalid fats value in BPB"));
        }

        if bpb.sectors_per_fat_16 == 0 {
            bpb.sectors_per_fat_32 = rdr.read_u32::<LittleEndian>()?;
            bpb.extended_flags = rdr.read_u16::<LittleEndian>()?;
            bpb.fs_version = rdr.read_u16::<LittleEndian>()?;
            bpb.root_dir_first_cluster = rdr.read_u32::<LittleEndian>()?;
            bpb.fs_info_sector = rdr.read_u16::<LittleEndian>()?;
            bpb.backup_boot_sector = rdr.read_u16::<LittleEndian>()?;
            rdr.read_exact(&mut bpb.reserved_0)?;
            bpb.drive_num = rdr.read_u8()?;
            bpb.reserved_1 = rdr.read_u8()?;
            bpb.ext_sig = rdr.read_u8()?; // 0x29
            bpb.volume_id = rdr.read_u32::<LittleEndian>()?;
            rdr.read_exact(&mut bpb.volume_label)?;
            rdr.read_exact(&mut bpb.fs_type_label)?;
        } else {
            bpb.drive_num = rdr.read_u8()?;
            bpb.reserved_1 = rdr.read_u8()?;
            bpb.ext_sig = rdr.read_u8()?; // 0x29
            bpb.volume_id = rdr.read_u32::<LittleEndian>()?;
            rdr.read_exact(&mut bpb.volume_label)?;
            rdr.read_exact(&mut bpb.fs_type_label)?;
        }
        Ok(bpb)
    }

    fn fat_type_from_clusters(total_clusters: u32) -> FatType {
        if total_clusters < 4085 {
            FatType::Fat12
        } else if total_clusters < 65525 {
            FatType::Fat16
        } else {
            FatType::Fat32
        }
    }

    fn read_boot_record(rdr: &mut Read) -> io::Result<BootRecord> {
        let mut boot: BootRecord = Default::default();
        rdr.read_exact(&mut boot.bootjmp)?;
        rdr.read_exact(&mut boot.oem_name)?;
        boot.bpb = Self::read_bpb(rdr)?;

        if boot.bpb.sectors_per_fat_16 == 0 {
            rdr.read_exact(&mut boot.boot_code[0..420])?;
        } else {
            rdr.read_exact(&mut boot.boot_code[0..448])?;
        }
        rdr.read_exact(&mut boot.boot_sig)?;
        Ok(boot)
    }

    pub(crate) fn offset_from_sector(&self, sector: u32) -> u64 {
        (sector as u64) * self.boot.bpb.bytes_per_sector as u64
    }

    pub(crate) fn sector_from_cluster(&self, cluster: u32) -> u32 {
        ((cluster - 2) * self.boot.bpb.sectors_per_cluster as u32) + self.first_data_sector
    }

    pub(crate) fn get_cluster_size(&self) -> u32 {
        self.boot.bpb.sectors_per_cluster as u32 * self.boot.bpb.bytes_per_sector as u32
    }

    pub(crate) fn offset_from_cluster(&self, cluser: u32) -> u64 {
        self.offset_from_sector(self.sector_from_cluster(cluser))
    }

    fn fat_slice<'b>(&'b self) -> DiskSlice<'b, 'a> {
        let sectors_per_fat =
            if self.boot.bpb.sectors_per_fat_16 == 0 { self.boot.bpb.sectors_per_fat_32 }
            else { self.boot.bpb.sectors_per_fat_16 as u32 };
        let mirroring_enabled = self.boot.bpb.extended_flags & 0x80 == 0;
        let (fat_first_sector, mirrors) = if mirroring_enabled {
            (self.boot.bpb.reserved_sectors as u32, self.boot.bpb.fats)
        } else {
            let active_fat = (self.boot.bpb.extended_flags & 0x0F) as u32;
            let fat_first_sector = (self.boot.bpb.reserved_sectors as u32) + active_fat * sectors_per_fat;
            (fat_first_sector, 1)
        };
        DiskSlice::from_sectors(fat_first_sector, sectors_per_fat, mirrors, self)
    }

    pub(crate) fn cluster_iter<'b>(&'b self, cluster: u32) -> ClusterIterator<'b, 'a> {
        let disk_slice = self.fat_slice();
        ClusterIterator::new(disk_slice, self.fat_type, cluster)
    }

    pub(crate) fn alloc_cluster(&self, prev_cluster: Option<u32>) -> io::Result<u32> {
        let mut disk_slice = self.fat_slice();
        alloc_cluster(&mut disk_slice, self.fat_type, prev_cluster)
    }
}

#[derive(Clone)]
pub(crate) struct DiskSlice<'a, 'b: 'a> {
    begin: u64,
    size: u64,
    offset: u64,
    mirrors: u8,
    fs: &'a FileSystem<'b>,
}

impl <'a, 'b> DiskSlice<'a, 'b> {
    pub(crate) fn new(begin: u64, size: u64, mirrors: u8, fs: FileSystemRef<'a, 'b>) -> Self {
        DiskSlice { begin, size, mirrors, fs, offset: 0 }
    }

    pub(crate) fn from_sectors(first_sector: u32, sector_count: u32, mirrors: u8, fs: FileSystemRef<'a, 'b>) -> Self {
        let bytes_per_sector = fs.boot.bpb.bytes_per_sector as u64;
        Self::new(first_sector as u64 * bytes_per_sector, sector_count as u64 * bytes_per_sector, mirrors, fs)
    }

    pub(crate) fn abs_pos(&self) -> u64 {
        self.begin + self.offset
    }
}

impl <'a, 'b> Read for DiskSlice<'a, 'b> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let offset = self.begin + self.offset;
        let read_size = cmp::min((self.size - self.offset) as usize, buf.len());
        let mut disk = self.fs.disk.borrow_mut();
        disk.seek(SeekFrom::Start(offset))?;
        let size = disk.read(&mut buf[..read_size])?;
        self.offset += size as u64;
        Ok(size)
    }
}

impl <'a, 'b> Write for DiskSlice<'a, 'b> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let offset = self.begin + self.offset;
        let write_size = cmp::min((self.size - self.offset) as usize, buf.len());
        for i in 0..self.mirrors {
            let mut disk = self.fs.disk.borrow_mut();
            disk.seek(SeekFrom::Start(offset + i as u64 * self.size))?;
            disk.write_all(&buf[..write_size])?;
        }
        self.offset += write_size as u64;
        Ok(write_size)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut disk = self.fs.disk.borrow_mut();
        disk.flush()
    }
}

impl <'a, 'b> Seek for DiskSlice<'a, 'b> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_offset = match pos {
            SeekFrom::Current(x) => self.offset as i64 + x,
            SeekFrom::Start(x) => x as i64,
            SeekFrom::End(x) => self.size as i64 + x,
        };
        if new_offset < 0 || new_offset as u64 > self.size {
            Err(io::Error::new(ErrorKind::InvalidInput, "invalid seek"))
        } else {
            self.offset = new_offset as u64;
            Ok(self.offset)
        }
    }
}
