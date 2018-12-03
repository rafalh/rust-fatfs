#[cfg(all(not(feature = "std"), feature = "alloc"))]
use alloc::String;
use core::cell::{Cell, RefCell};
use core::char;
use core::cmp;
use core::fmt::Debug;
use core::iter::FromIterator;
use io;
use io::prelude::*;
use io::{Error, ErrorKind, SeekFrom};

use byteorder::LittleEndian;
use byteorder_ext::{ReadBytesExt, WriteBytesExt};

use dir::{Dir, DirRawStream};
use dir_entry::DIR_ENTRY_SIZE;
use file::File;
use table::{alloc_cluster, count_free_clusters, read_fat_flags, format_fat, ClusterIterator, RESERVED_FAT_ENTRIES};
use time::{TimeProvider, DEFAULT_TIME_PROVIDER};

// FAT implementation based on:
//   http://wiki.osdev.org/FAT
//   https://www.win.tue.nl/~aeb/linux/fs/fat/fat-1.html

/// A type of FAT filesystem.
///
/// `FatType` values are based on the size of File Allocation Table entry.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum FatType {
    /// 12 bits per FAT entry
    Fat12,
    /// 16 bits per FAT entry
    Fat16,
    /// 32 bits per FAT entry
    Fat32,
}

impl FatType {
    fn from_clusters(total_clusters: u32) -> FatType {
        if total_clusters < 4085 {
            FatType::Fat12
        } else if total_clusters < 65525 {
            FatType::Fat16
        } else {
            FatType::Fat32
        }
    }

    pub(crate) fn bits_per_fat_entry(&self) -> u32 {
        match self {
            FatType::Fat12 => 12,
            FatType::Fat16 => 16,
            FatType::Fat32 => 32,
        }
    }
}

/// A FAT volume status flags retrived from the Boot Sector and the allocation table second entry.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct FsStatusFlags {
    pub(crate) dirty: bool,
    pub(crate) io_error: bool,
}

impl FsStatusFlags {
    /// Checks if the volume is marked as dirty.
    ///
    /// Dirty flag means volume has been suddenly ejected from filesystem without unmounting.
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// Checks if the volume has the IO Error flag active.
    pub fn io_error(&self) -> bool {
        self.io_error
    }

    fn encode(&self) -> u8 {
        let mut res = 0u8;
        if self.dirty {
            res |= 1;
        }
        if self.io_error {
            res |= 2;
        }
        res
    }

    fn decode(flags: u8) -> Self {
        FsStatusFlags {
            dirty: flags & 1 != 0,
            io_error: flags & 2 != 0,
        }
    }
}

/// A sum of `Read` and `Seek` traits.
pub trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

/// A sum of `Read`, `Write` and `Seek` traits.
pub trait ReadWriteSeek: Read + Write + Seek {}
impl<T: Read + Write + Seek> ReadWriteSeek for T {}

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

impl BiosParameterBlock {
    fn deserialize<T: Read>(rdr: &mut T) -> io::Result<BiosParameterBlock> {
        let mut bpb: BiosParameterBlock = Default::default();
        bpb.bytes_per_sector = rdr.read_u16::<LittleEndian>()?;
        bpb.sectors_per_cluster = rdr.read_u8()?;
        bpb.reserved_sectors = rdr.read_u16::<LittleEndian>()?;
        bpb.fats = rdr.read_u8()?;
        bpb.root_entries = rdr.read_u16::<LittleEndian>()?;
        bpb.total_sectors_16 = rdr.read_u16::<LittleEndian>()?;
        bpb.media = rdr.read_u8()?;
        bpb.sectors_per_fat_16 = rdr.read_u16::<LittleEndian>()?;
        bpb.sectors_per_track = rdr.read_u16::<LittleEndian>()?;
        bpb.heads = rdr.read_u16::<LittleEndian>()?;
        bpb.hidden_sectors = rdr.read_u32::<LittleEndian>()?;
        bpb.total_sectors_32 = rdr.read_u32::<LittleEndian>()?;

        if bpb.is_fat32() {
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

        // when the extended boot signature is anything other than 0x29, the fields are invalid
        if bpb.ext_sig != 0x29 {
            // fields after ext_sig are not used - clean them
            bpb.volume_id = 0;
            bpb.volume_label = [0; 11];
            bpb.fs_type_label = [0; 8];
        }

        Ok(bpb)
    }

    fn serialize<T: Write>(&self, mut wrt: T) -> io::Result<()> {
        wrt.write_u16::<LittleEndian>(self.bytes_per_sector)?;
        wrt.write_u8(self.sectors_per_cluster)?;
        wrt.write_u16::<LittleEndian>(self.reserved_sectors)?;
        wrt.write_u8(self.fats)?;
        wrt.write_u16::<LittleEndian>(self.root_entries)?;
        wrt.write_u16::<LittleEndian>(self.total_sectors_16)?;
        wrt.write_u8(self.media)?;
        wrt.write_u16::<LittleEndian>(self.sectors_per_fat_16)?;
        wrt.write_u16::<LittleEndian>(self.sectors_per_track)?;
        wrt.write_u16::<LittleEndian>(self.heads)?;
        wrt.write_u32::<LittleEndian>(self.hidden_sectors)?;
        wrt.write_u32::<LittleEndian>(self.total_sectors_32)?;

        if self.is_fat32() {
            wrt.write_u32::<LittleEndian>(self.sectors_per_fat_32)?;
            wrt.write_u16::<LittleEndian>(self.extended_flags)?;
            wrt.write_u16::<LittleEndian>(self.fs_version)?;
            wrt.write_u32::<LittleEndian>(self.root_dir_first_cluster)?;
            wrt.write_u16::<LittleEndian>(self.fs_info_sector)?;
            wrt.write_u16::<LittleEndian>(self.backup_boot_sector)?;
            wrt.write_all(&self.reserved_0)?;
            wrt.write_u8(self.drive_num)?;
            wrt.write_u8(self.reserved_1)?;
            wrt.write_u8(self.ext_sig)?; // 0x29
            wrt.write_u32::<LittleEndian>(self.volume_id)?;
            wrt.write_all(&self.volume_label)?;
            wrt.write_all(&self.fs_type_label)?;
        } else {
            wrt.write_u8(self.drive_num)?;
            wrt.write_u8(self.reserved_1)?;
            wrt.write_u8(self.ext_sig)?; // 0x29
            wrt.write_u32::<LittleEndian>(self.volume_id)?;
            wrt.write_all(&self.volume_label)?;
            wrt.write_all(&self.fs_type_label)?;
        }
        Ok(())
    }

    fn validate(&self) -> io::Result<()> {
        // sanity checks
        if self.bytes_per_sector.count_ones() != 1 {
            return Err(Error::new(
                ErrorKind::Other,
                "invalid bytes_per_sector value in BPB (not power of two)",
            ));
        } else if self.bytes_per_sector < 512 {
            return Err(Error::new(ErrorKind::Other, "invalid bytes_per_sector value in BPB (value < 512)"));
        } else if self.bytes_per_sector > 4096 {
            return Err(Error::new(ErrorKind::Other, "invalid bytes_per_sector value in BPB (value > 4096)"));
        }

        if self.sectors_per_cluster.count_ones() != 1 {
            return Err(Error::new(
                ErrorKind::Other,
                "invalid sectors_per_cluster value in BPB (not power of two)",
            ));
        } else if self.sectors_per_cluster < 1 {
            return Err(Error::new(ErrorKind::Other, "invalid sectors_per_cluster value in BPB (value < 1)"));
        } else if self.sectors_per_cluster > 128 {
            return Err(Error::new(
                ErrorKind::Other,
                "invalid sectors_per_cluster value in BPB (value > 128)",
            ));
        }

        // bytes per sector is u16, sectors per cluster is u8, so guaranteed no overflow in multiplication
        let bytes_per_cluster = self.bytes_per_sector as u32 * self.sectors_per_cluster as u32;
        let maximum_compatibility_bytes_per_cluster: u32 = 32 * 1024;

        if bytes_per_cluster > maximum_compatibility_bytes_per_cluster {
            // 32k is the largest value to maintain greatest compatibility
            // Many implementations appear to support 64k per cluster, and some may support 128k or larger
            // However, >32k is not as thoroughly tested...
            warn!("fs compatibility: bytes_per_cluster value '{}' in BPB exceeds '{}', and thus may be incompatible with some implementations",
                bytes_per_cluster, maximum_compatibility_bytes_per_cluster);
        }

        let is_fat32 = self.is_fat32();
        if self.reserved_sectors < 1 {
            return Err(Error::new(ErrorKind::Other, "invalid reserved_sectors value in BPB"));
        } else if !is_fat32 && self.reserved_sectors != 1 {
            // Microsoft document indicates fat12 and fat16 code exists that presume this value is 1
            warn!(
                "fs compatibility: reserved_sectors value '{}' in BPB is not '1', and thus is incompatible with some implementations",
                self.reserved_sectors
            );
        }

        if self.fats == 0 {
            return Err(Error::new(ErrorKind::Other, "invalid fats value in BPB"));
        } else if self.fats > 2 {
            // Microsoft document indicates that few implementations support any values other than 1 or 2
            warn!(
                "fs compatibility: numbers of FATs '{}' in BPB is greater than '2', and thus is incompatible with some implementations",
                self.fats
            );
        }

        if is_fat32 && self.root_entries != 0 {
            return Err(Error::new(
                ErrorKind::Other,
                "Invalid root_entries value in BPB (should be zero for FAT32)",
            ));
        }

        if is_fat32 && self.total_sectors_16 != 0 {
            return Err(Error::new(
                ErrorKind::Other,
                "Invalid total_sectors_16 value in BPB (should be zero for FAT32)",
            ));
        }

        if (self.total_sectors_16 == 0) == (self.total_sectors_32 == 0) {
            return Err(Error::new(
                ErrorKind::Other,
                "Invalid BPB (total_sectors_16 or total_sectors_32 should be non-zero)",
            ));
        }

        if is_fat32 && self.sectors_per_fat_32 == 0 {
            return Err(Error::new(
                ErrorKind::Other,
                "Invalid sectors_per_fat_32 value in BPB (should be non-zero for FAT32)",
            ));
        }

        if self.fs_version != 0 {
            return Err(Error::new(ErrorKind::Other, "Unknown FS version"));
        }

        if self.total_sectors() <= self.first_data_sector() {
            return Err(Error::new(
                ErrorKind::Other,
                "Invalid BPB (total_sectors field value is too small)",
            ));
        }

        let total_clusters = self.total_clusters();
        let fat_type = FatType::from_clusters(total_clusters);
        if is_fat32 != (fat_type == FatType::Fat32) {
            return Err(Error::new(
                ErrorKind::Other,
                "Invalid BPB (result of FAT32 determination from total number of clusters and sectors_per_fat_16 field differs)",
            ));
        }

        let fat_entries_per_sector = self.fat_entries_per_sector(fat_type);
        let total_fat_entries = self.sectors_per_fat() * fat_entries_per_sector as u32;
        if total_fat_entries - RESERVED_FAT_ENTRIES < total_clusters {
            warn!("FAT is too small to compared to total number of clusters");
        }

        Ok(())
    }

    fn mirroring_enabled(&self) -> bool {
        self.extended_flags & 0x80 == 0
    }

    fn active_fat(&self) -> u16 {
        // The zero-based number of the active FAT is only valid if mirroring is disabled.
        if self.mirroring_enabled() {
            0
        } else {
            self.extended_flags & 0x0F
        }
    }

    fn status_flags(&self) -> FsStatusFlags {
        FsStatusFlags::decode(self.reserved_1)
    }

    fn is_fat32(&self) -> bool {
        // because this field must be zero on FAT32, and
        // because it must be non-zero on FAT12/FAT16,
        // this provides a simple way to detect FAT32
        self.sectors_per_fat_16 == 0
    }

    fn sectors_per_fat(&self) -> u32 {
        if self.is_fat32() {
            self.sectors_per_fat_32
        } else {
            self.sectors_per_fat_16 as u32
        }
    }

    fn total_sectors(&self) -> u32 {
        if self.total_sectors_16 == 0 {
            self.total_sectors_32
        } else {
            self.total_sectors_16 as u32
        }
    }

    fn root_dir_sectors(&self) -> u32 {
        let root_dir_bytes = self.root_entries as u32 * DIR_ENTRY_SIZE as u32;
        (root_dir_bytes + self.bytes_per_sector as u32 - 1) / self.bytes_per_sector as u32
    }

    fn sectors_per_all_fats(&self) -> u32 {
        self.fats as u32 * self.sectors_per_fat()
    }

    fn first_data_sector(&self) -> u32 {
        let root_dir_sectors = self.root_dir_sectors();
        let fat_sectors = self.sectors_per_all_fats();
        self.reserved_sectors as u32 + fat_sectors + root_dir_sectors
    }

    fn total_clusters(&self) -> u32 {
        let total_sectors = self.total_sectors();
        let first_data_sector = self.first_data_sector();
        let data_sectors = total_sectors - first_data_sector;
        data_sectors / self.sectors_per_cluster as u32
    }

    fn fat_entries_per_sector(&self, fat_type: FatType) -> u16 {
        match fat_type {
            FatType::Fat12 => self.bytes_per_sector * 8 / 12,
            FatType::Fat16 => self.bytes_per_sector * 8 / 16,
            FatType::Fat32 => self.bytes_per_sector * 8 / 32,
        }
    }
}

#[allow(dead_code)]
struct BootRecord {
    bootjmp: [u8; 3],
    oem_name: [u8; 8],
    bpb: BiosParameterBlock,
    boot_code: [u8; 448],
    boot_sig: [u8; 2],
}

impl BootRecord {
    fn deserialize<T: Read>(rdr: &mut T) -> io::Result<BootRecord> {
        let mut boot: BootRecord = Default::default();
        rdr.read_exact(&mut boot.bootjmp)?;
        rdr.read_exact(&mut boot.oem_name)?;
        boot.bpb = BiosParameterBlock::deserialize(rdr)?;

        if boot.bpb.is_fat32() {
            rdr.read_exact(&mut boot.boot_code[0..420])?;
        } else {
            rdr.read_exact(&mut boot.boot_code[0..448])?;
        }
        rdr.read_exact(&mut boot.boot_sig)?;
        Ok(boot)
    }

    fn serialize<T: Write>(&self, mut wrt: T) -> io::Result<()> {
        wrt.write_all(&self.bootjmp)?;
        wrt.write_all(&self.oem_name)?;
        self.bpb.serialize(&mut wrt)?;

        if self.bpb.is_fat32() {
            wrt.write_all(&self.boot_code[0..420])?;
        } else {
            wrt.write_all(&self.boot_code[0..448])?;
        }
        wrt.write_all(&self.boot_sig)?;
        Ok(())
    }

    fn validate(&self) -> io::Result<()> {
        if self.boot_sig != [0x55, 0xAA] {
            return Err(Error::new(ErrorKind::Other, "Invalid boot sector signature"));
        }
        if self.bootjmp[0] != 0xEB && self.bootjmp[0] != 0xE9 {
            warn!("Unknown opcode {:x} in bootjmp boot sector field", self.bootjmp[0]);
        }
        self.bpb.validate()?;
        Ok(())
    }
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

#[derive(Clone, Default, Debug)]
struct FsInfoSector {
    free_cluster_count: Option<u32>,
    next_free_cluster: Option<u32>,
    dirty: bool,
}

impl FsInfoSector {
    const LEAD_SIG: u32 = 0x41615252;
    const STRUC_SIG: u32 = 0x61417272;
    const TRAIL_SIG: u32 = 0xAA550000;

    fn deserialize<T: Read>(rdr: &mut T) -> io::Result<FsInfoSector> {
        let lead_sig = rdr.read_u32::<LittleEndian>()?;
        if lead_sig != Self::LEAD_SIG {
            return Err(Error::new(ErrorKind::Other, "invalid lead_sig in FsInfo sector"));
        }
        let mut reserved = [0u8; 480];
        rdr.read_exact(&mut reserved)?;
        let struc_sig = rdr.read_u32::<LittleEndian>()?;
        if struc_sig != Self::STRUC_SIG {
            return Err(Error::new(ErrorKind::Other, "invalid struc_sig in FsInfo sector"));
        }
        let free_cluster_count = match rdr.read_u32::<LittleEndian>()? {
            0xFFFFFFFF => None,
            // Note: value is validated in FileSystem::new function using values from BPB
            n => Some(n),
        };
        let next_free_cluster = match rdr.read_u32::<LittleEndian>()? {
            0xFFFFFFFF => None,
            0 | 1 => {
                warn!("invalid next_free_cluster in FsInfo sector (values 0 and 1 are reserved)");
                None
            },
            // Note: other values are validated in FileSystem::new function using values from BPB
            n => Some(n),
        };
        let mut reserved2 = [0u8; 12];
        rdr.read_exact(&mut reserved2)?;
        let trail_sig = rdr.read_u32::<LittleEndian>()?;
        if trail_sig != Self::TRAIL_SIG {
            return Err(Error::new(ErrorKind::Other, "invalid trail_sig in FsInfo sector"));
        }
        Ok(FsInfoSector {
            free_cluster_count,
            next_free_cluster,
            dirty: false,
        })
    }

    fn serialize<T: Write>(&self, wrt: &mut T) -> io::Result<()> {
        wrt.write_u32::<LittleEndian>(Self::LEAD_SIG)?;
        let reserved = [0u8; 480];
        wrt.write(&reserved)?;
        wrt.write_u32::<LittleEndian>(Self::STRUC_SIG)?;
        wrt.write_u32::<LittleEndian>(self.free_cluster_count.unwrap_or(0xFFFFFFFF))?;
        wrt.write_u32::<LittleEndian>(self.next_free_cluster.unwrap_or(0xFFFFFFFF))?;
        let reserved2 = [0u8; 12];
        wrt.write(&reserved2)?;
        wrt.write_u32::<LittleEndian>(Self::TRAIL_SIG)?;
        Ok(())
    }

    fn validate_and_fix(&mut self, total_clusters: u32) {
        let max_valid_cluster_number = total_clusters + RESERVED_FAT_ENTRIES;
        if let Some(n) = self.free_cluster_count {
            if n > total_clusters {
                warn!(
                    "invalid free_cluster_count ({}) in fs_info exceeds total cluster count ({})",
                    n, total_clusters
                );
                self.free_cluster_count = None;
            }
        }
        if let Some(n) = self.next_free_cluster {
            if n > max_valid_cluster_number {
                warn!(
                    "invalid free_cluster_count ({}) in fs_info exceeds maximum cluster number ({})",
                    n, max_valid_cluster_number
                );
                self.next_free_cluster = None;
            }
        }
    }

    fn add_free_clusters(&mut self, free_clusters: i32) {
        if let Some(n) = self.free_cluster_count {
            self.free_cluster_count = Some((n as i32 + free_clusters) as u32);
            self.dirty = true;
        }
    }

    fn set_next_free_cluster(&mut self, cluster: u32) {
        self.next_free_cluster = Some(cluster);
        self.dirty = true;
    }

    fn set_free_cluster_count(&mut self, free_cluster_count: u32) {
        self.free_cluster_count = Some(free_cluster_count);
        self.dirty = true;
    }
}

/// A FAT filesystem mount options.
///
/// Options are specified as an argument for `FileSystem::new` method.
#[derive(Copy, Clone, Debug)]
pub struct FsOptions {
    pub(crate) update_accessed_date: bool,
    pub(crate) oem_cp_converter: &'static OemCpConverter,
    pub(crate) time_provider: &'static TimeProvider,
}

impl FsOptions {
    /// Creates a `FsOptions` struct with default options.
    pub fn new() -> Self {
        FsOptions {
            update_accessed_date: false,
            oem_cp_converter: &LOSSY_OEM_CP_CONVERTER,
            time_provider: &DEFAULT_TIME_PROVIDER,
        }
    }

    /// If enabled accessed date field in directory entry is updated when reading or writing a file.
    pub fn update_accessed_date(mut self, enabled: bool) -> Self {
        self.update_accessed_date = enabled;
        self
    }

    /// Changes default OEM code page encoder-decoder.
    pub fn oem_cp_converter(mut self, oem_cp_converter: &'static OemCpConverter) -> Self {
        self.oem_cp_converter = oem_cp_converter;
        self
    }

    /// Changes default time provider.
    pub fn time_provider(mut self, time_provider: &'static TimeProvider) -> Self {
        self.time_provider = time_provider;
        self
    }
}

/// A FAT volume statistics.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct FileSystemStats {
    cluster_size: u32,
    total_clusters: u32,
    free_clusters: u32,
}

impl FileSystemStats {
    /// Cluster size in bytes
    pub fn cluster_size(&self) -> u32 {
        self.cluster_size
    }

    /// Number of total clusters in filesystem usable for file allocation
    pub fn total_clusters(&self) -> u32 {
        self.total_clusters
    }

    /// Number of free clusters
    pub fn free_clusters(&self) -> u32 {
        self.free_clusters
    }
}

/// A FAT filesystem object.
///
/// `FileSystem` struct is representing a state of a mounted FAT volume.
pub struct FileSystem<T: ReadWriteSeek> {
    pub(crate) disk: RefCell<T>,
    pub(crate) options: FsOptions,
    fat_type: FatType,
    bpb: BiosParameterBlock,
    first_data_sector: u32,
    root_dir_sectors: u32,
    total_clusters: u32,
    fs_info: RefCell<FsInfoSector>,
    current_status_flags: Cell<FsStatusFlags>,
}

impl<T: ReadWriteSeek> FileSystem<T> {
    /// Creates a new filesystem object instance.
    ///
    /// Supplied `disk` parameter cannot be seeked. If there is a need to read a fragment of disk
    /// image (e.g. partition) library user should wrap the file handle in a struct limiting
    /// access to partition bytes only e.g. `fscommon::StreamSlice`.
    ///
    /// Note: creating multiple filesystem objects with one underlying device/disk image can
    /// cause a filesystem corruption.
    pub fn new(mut disk: T, options: FsOptions) -> io::Result<Self> {
        // Make sure given image is not seeked
        debug_assert!(disk.seek(SeekFrom::Current(0))? == 0);

        // read boot sector
        let bpb = {
            let boot = BootRecord::deserialize(&mut disk)?;
            boot.validate()?;
            boot.bpb
        };

        let root_dir_sectors = bpb.root_dir_sectors();
        let first_data_sector = bpb.first_data_sector();
        let total_clusters = bpb.total_clusters();
        let fat_type = FatType::from_clusters(total_clusters);

        // read FSInfo sector if this is FAT32
        let mut fs_info = if fat_type == FatType::Fat32 {
            disk.seek(SeekFrom::Start(bpb.fs_info_sector as u64 * bpb.bytes_per_sector as u64))?;
            FsInfoSector::deserialize(&mut disk)?
        } else {
            FsInfoSector::default()
        };

        // if dirty flag is set completly ignore free_cluster_count in FSInfo
        if bpb.status_flags().dirty {
            fs_info.free_cluster_count = None;
        }

        // Validate the numbers stored in the free_cluster_count and next_free_cluster are within bounds for volume
        fs_info.validate_and_fix(total_clusters);

        // return FileSystem struct
        let status_flags = bpb.status_flags();
        Ok(FileSystem {
            disk: RefCell::new(disk),
            options,
            fat_type,
            bpb,
            first_data_sector,
            root_dir_sectors,
            total_clusters,
            fs_info: RefCell::new(fs_info),
            current_status_flags: Cell::new(status_flags),
        })
    }

    /// Returns a type of File Allocation Table (FAT) used by this filesystem.
    pub fn fat_type(&self) -> FatType {
        self.fat_type
    }

    /// Returns a volume identifier read from BPB in the Boot Sector.
    pub fn volume_id(&self) -> u32 {
        self.bpb.volume_id
    }

    /// Returns a volume label from BPB in the Boot Sector as `String`.
    ///
    /// Non-ASCII characters are replaced by the replacement character (U+FFFD).
    /// Note: File with `VOLUME_ID` attribute in root directory is ignored by this library.
    /// Only label from BPB is used.
    #[cfg(feature = "alloc")]
    pub fn volume_label(&self) -> String {
        // Decode volume label from OEM codepage
        let volume_label_iter = self.volume_label_as_bytes().iter().cloned();
        let char_iter = volume_label_iter.map(|c| self.options.oem_cp_converter.decode(c));
        // Build string from character iterator
        String::from_iter(char_iter)
    }

    /// Returns a volume label from BPB in the Boot Sector as byte array slice.
    ///
    /// Label is encoded in the OEM codepage.
    /// Note: File with `VOLUME_ID` attribute in root directory is ignored by this library.
    /// Only label from BPB is used.
    pub fn volume_label_as_bytes(&self) -> &[u8] {
        const PADDING: u8 = 0x20;
        let full_label_slice = &self.bpb.volume_label;
        let len = full_label_slice.iter().rposition(|b| *b != PADDING).map(|p| p + 1).unwrap_or(0);
        &full_label_slice[..len]
    }

    /// Returns a volume label from root directory as `String`.
    ///
    /// It finds file with `VOLUME_ID` attribute and returns its short name.
    #[cfg(feature = "alloc")]
    pub fn read_volume_label_from_root_dir(&self) -> io::Result<Option<String>> {
        // Note: DirEntry::file_short_name() cannot be used because it interprets name as 8.3
        // (adds dot before an extension)
        let volume_label_opt = self.read_volume_label_from_root_dir_as_bytes()?;
        if let Some(volume_label) = volume_label_opt {
            const PADDING: u8 = 0x20;
            // Strip label padding
            let len = volume_label.iter().rposition(|b| *b != PADDING).map(|p| p + 1).unwrap_or(0);
            let label_slice = &volume_label[..len];
            // Decode volume label from OEM codepage
            let volume_label_iter = label_slice.iter().cloned();
            let char_iter = volume_label_iter.map(|c| self.options.oem_cp_converter.decode(c));
            // Build string from character iterator
            Ok(Some(String::from_iter(char_iter)))
        } else {
            Ok(None)
        }
    }

    /// Returns a volume label from root directory as byte array.
    ///
    /// Label is encoded in the OEM codepage.
    /// It finds file with `VOLUME_ID` attribute and returns its short name.
    pub fn read_volume_label_from_root_dir_as_bytes(&self) -> io::Result<Option<[u8; 11]>> {
        let entry_opt = self.root_dir().find_volume_entry()?;
        Ok(entry_opt.map(|e| *e.raw_short_name()))
    }

    /// Returns a root directory object allowing for futher penetration of a filesystem structure.
    pub fn root_dir<'b>(&'b self) -> Dir<'b, T> {
        let root_rdr = {
            match self.fat_type {
                FatType::Fat12 | FatType::Fat16 => DirRawStream::Root(DiskSlice::from_sectors(
                    self.first_data_sector - self.root_dir_sectors,
                    self.root_dir_sectors,
                    1,
                    &self.bpb,
                    FsIoAdapter { fs: self },
                )),
                _ => DirRawStream::File(File::new(Some(self.bpb.root_dir_first_cluster), None, self)),
            }
        };
        Dir::new(root_rdr, self)
    }

    fn offset_from_sector(&self, sector: u32) -> u64 {
        (sector as u64) * self.bpb.bytes_per_sector as u64
    }

    fn sector_from_cluster(&self, cluster: u32) -> u32 {
        ((cluster - 2) * self.bpb.sectors_per_cluster as u32) + self.first_data_sector
    }

    pub(crate) fn cluster_size(&self) -> u32 {
        self.bpb.sectors_per_cluster as u32 * self.bpb.bytes_per_sector as u32
    }

    pub(crate) fn offset_from_cluster(&self, cluser: u32) -> u64 {
        self.offset_from_sector(self.sector_from_cluster(cluser))
    }

    fn fat_slice<'b>(&'b self) -> DiskSlice<FsIoAdapter<'b, T>> {
        let io = FsIoAdapter {
            fs: self,
        };
        fat_slice(io, &self.bpb)
    }

    pub(crate) fn cluster_iter<'b>(&'b self, cluster: u32) -> ClusterIterator<DiskSlice<FsIoAdapter<'b, T>>> {
        let disk_slice = self.fat_slice();
        ClusterIterator::new(disk_slice, self.fat_type, cluster)
    }

    pub(crate) fn truncate_cluster_chain(&self, cluster: u32) -> io::Result<()> {
        let mut iter = self.cluster_iter(cluster);
        let num_free = iter.truncate()?;
        let mut fs_info = self.fs_info.borrow_mut();
        fs_info.add_free_clusters(num_free as i32);
        Ok(())
    }

    pub(crate) fn free_cluster_chain(&self, cluster: u32) -> io::Result<()> {
        let mut iter = self.cluster_iter(cluster);
        let num_free = iter.free()?;
        let mut fs_info = self.fs_info.borrow_mut();
        fs_info.add_free_clusters(num_free as i32);
        Ok(())
    }

    pub(crate) fn alloc_cluster(&self, prev_cluster: Option<u32>) -> io::Result<u32> {
        let hint = self.fs_info.borrow().next_free_cluster;
        let mut fat = self.fat_slice();
        let cluster = alloc_cluster(&mut fat, self.fat_type, prev_cluster, hint, self.total_clusters)?;
        let mut fs_info = self.fs_info.borrow_mut();
        fs_info.set_next_free_cluster(cluster + 1);
        fs_info.add_free_clusters(-1);
        Ok(cluster)
    }

    /// Returns status flags for this volume.
    pub fn read_status_flags(&self) -> io::Result<FsStatusFlags> {
        let bpb_status = self.bpb.status_flags();
        let fat_status = read_fat_flags(&mut self.fat_slice(), self.fat_type)?;
        Ok(FsStatusFlags {
            dirty: bpb_status.dirty || fat_status.dirty,
            io_error: bpb_status.io_error || fat_status.io_error,
        })
    }

    /// Returns filesystem statistics like number of total and free clusters.
    ///
    /// For FAT32 volumes number of free clusters from FSInfo sector is returned (may be incorrect).
    /// For other FAT variants number is computed on the first call to this method and cached for later use.
    pub fn stats(&self) -> io::Result<FileSystemStats> {
        let free_clusters_option = self.fs_info.borrow().free_cluster_count;
        let free_clusters = match free_clusters_option {
            Some(n) => n,
            _ => self.recalc_free_clusters()?,
        };
        Ok(FileSystemStats {
            cluster_size: self.cluster_size(),
            total_clusters: self.total_clusters,
            free_clusters,
        })
    }

    /// Forces free clusters recalculation.
    fn recalc_free_clusters(&self) -> io::Result<u32> {
        let mut fat = self.fat_slice();
        let free_cluster_count = count_free_clusters(&mut fat, self.fat_type, self.total_clusters)?;
        self.fs_info.borrow_mut().set_free_cluster_count(free_cluster_count);
        Ok(free_cluster_count)
    }

    /// Unmounts the filesystem.
    ///
    /// Updates FSInfo sector if needed.
    pub fn unmount(self) -> io::Result<()> {
        self.unmount_internal()
    }

    fn unmount_internal(&self) -> io::Result<()> {
        self.flush_fs_info()?;
        self.set_dirty_flag(false)?;
        Ok(())
    }

    fn flush_fs_info(&self) -> io::Result<()> {
        let mut fs_info = self.fs_info.borrow_mut();
        if self.fat_type == FatType::Fat32 && fs_info.dirty {
            let mut disk = self.disk.borrow_mut();
            disk.seek(SeekFrom::Start(self.offset_from_sector(self.bpb.fs_info_sector as u32)))?;
            fs_info.serialize(&mut *disk)?;
            fs_info.dirty = false;
        }
        Ok(())
    }

    pub(crate) fn set_dirty_flag(&self, dirty: bool) -> io::Result<()> {
        // Do not overwrite flags read from BPB on mount
        let mut flags = self.bpb.status_flags();
        flags.dirty |= dirty;
        // Check if flags has changed
        let current_flags = self.current_status_flags.get();
        if flags == current_flags {
            // Nothing to do
            return Ok(());
        }
        let encoded = flags.encode();
        // Note: only one field is written to avoid rewriting entire boot-sector which could be dangerous
        // Compute reserver_1 field offset and write new flags
        let offset = if self.fat_type() == FatType::Fat32 { 0x041 } else { 0x025 };
        let mut disk = self.disk.borrow_mut();
        disk.seek(io::SeekFrom::Start(offset))?;
        disk.write_u8(encoded)?;
        self.current_status_flags.set(flags);
        Ok(())
    }
}

/// `Drop` implementation tries to unmount the filesystem when dropping.
impl<T: ReadWriteSeek> Drop for FileSystem<T> {
    fn drop(&mut self) {
        if let Err(err) = self.unmount_internal() {
            error!("unmount failed {}", err);
        }
    }
}

pub(crate) struct FsIoAdapter<'a, T: ReadWriteSeek + 'a> {
    fs: &'a FileSystem<T>,
}

impl<'a, T: ReadWriteSeek> Read for FsIoAdapter<'a, T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.fs.disk.borrow_mut().read(buf)
    }
}

impl<'a, T: ReadWriteSeek> Write for FsIoAdapter<'a, T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let size = self.fs.disk.borrow_mut().write(buf)?;
        if size > 0 {
            self.fs.set_dirty_flag(true)?;
        }
        Ok(size)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.fs.disk.borrow_mut().flush()
    }
}

impl<'a, T: ReadWriteSeek> Seek for FsIoAdapter<'a, T> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.fs.disk.borrow_mut().seek(pos)
    }
}

// Note: derive cannot be used because of invalid bounds. See: https://github.com/rust-lang/rust/issues/26925
impl<'a, T: ReadWriteSeek> Clone for FsIoAdapter<'a, T> {
    fn clone(&self) -> Self {
        FsIoAdapter {
            fs: self.fs,
        }
    }
}

fn fat_slice<T: ReadWriteSeek>(io: T, bpb: &BiosParameterBlock) -> DiskSlice<T> {
    let sectors_per_fat = bpb.sectors_per_fat();
    let mirroring_enabled = bpb.mirroring_enabled();
    let (fat_first_sector, mirrors) = if mirroring_enabled {
        (bpb.reserved_sectors as u32, bpb.fats)
    } else {
        let active_fat = bpb.active_fat() as u32;
        let fat_first_sector = (bpb.reserved_sectors as u32) + active_fat * sectors_per_fat;
        (fat_first_sector, 1)
    };
    DiskSlice::from_sectors(fat_first_sector, sectors_per_fat, mirrors, bpb, io)
}

pub(crate) struct DiskSlice<T> {
    begin: u64,
    size: u64,
    offset: u64,
    mirrors: u8,
    inner: T,
}

impl<T> DiskSlice<T> {
    pub(crate) fn new(begin: u64, size: u64, mirrors: u8, inner: T) -> Self {
        DiskSlice {
            begin,
            size,
            mirrors,
            inner,
            offset: 0,
        }
    }

    fn from_sectors(first_sector: u32, sector_count: u32, mirrors: u8, bpb: &BiosParameterBlock, inner: T) -> Self {
        let bytes_per_sector = bpb.bytes_per_sector as u64;
        Self::new(
            first_sector as u64 * bytes_per_sector,
            sector_count as u64 * bytes_per_sector,
            mirrors,
            inner,
        )
    }

    pub(crate) fn abs_pos(&self) -> u64 {
        self.begin + self.offset
    }
}

// Note: derive cannot be used because of invalid bounds. See: https://github.com/rust-lang/rust/issues/26925
impl<T: Clone> Clone for DiskSlice<T> {
    fn clone(&self) -> Self {
        DiskSlice {
            begin: self.begin,
            size: self.size,
            offset: self.offset,
            mirrors: self.mirrors,
            inner: self.inner.clone(),
        }
    }
}

impl<'a, T: Read + Seek> Read for DiskSlice<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let offset = self.begin + self.offset;
        let read_size = cmp::min((self.size - self.offset) as usize, buf.len());
        self.inner.seek(SeekFrom::Start(offset))?;
        let size = self.inner.read(&mut buf[..read_size])?;
        self.offset += size as u64;
        Ok(size)
    }
}

impl<'a, T: Write + Seek> Write for DiskSlice<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let offset = self.begin + self.offset;
        let write_size = cmp::min((self.size - self.offset) as usize, buf.len());
        if write_size == 0 {
            return Ok(0);
        }
        // Write data
        for i in 0..self.mirrors {
            self.inner.seek(SeekFrom::Start(offset + i as u64 * self.size))?;
            self.inner.write_all(&buf[..write_size])?;
        }
        self.offset += write_size as u64;
        Ok(write_size)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl<'a, T> Seek for DiskSlice<T> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_offset = match pos {
            SeekFrom::Current(x) => self.offset as i64 + x,
            SeekFrom::Start(x) => x as i64,
            SeekFrom::End(x) => self.size as i64 + x,
        };
        if new_offset < 0 || new_offset as u64 > self.size {
            Err(io::Error::new(ErrorKind::InvalidInput, "Seek to a negative offset"))
        } else {
            self.offset = new_offset as u64;
            Ok(self.offset)
        }
    }
}

/// An OEM code page encoder/decoder.
///
/// Provides a custom implementation for a short name encoding/decoding.
/// Default implementation changes all non-ASCII characters to the replacement character (U+FFFD).
/// `OemCpConverter` is specified by the `oem_cp_converter` property in `FsOptions` struct.
pub trait OemCpConverter: Debug {
    fn decode(&self, oem_char: u8) -> char;
    fn encode(&self, uni_char: char) -> Option<u8>;
}

#[derive(Debug)]
pub(crate) struct LossyOemCpConverter {
    _dummy: (),
}

impl OemCpConverter for LossyOemCpConverter {
    fn decode(&self, oem_char: u8) -> char {
        if oem_char <= 0x7F {
            oem_char as char
        } else {
            '\u{FFFD}'
        }
    }
    fn encode(&self, uni_char: char) -> Option<u8> {
        if uni_char <= '\x7F' {
            Some(uni_char as u8)
        } else {
            None
        }
    }
}

pub(crate) static LOSSY_OEM_CP_CONVERTER: LossyOemCpConverter = LossyOemCpConverter { _dummy: () };


#[derive(Default, Debug, Clone)]
pub struct FormatOptions {
    pub bytes_per_sector: Option<u16>,
    pub total_sectors: u32,
    pub bytes_per_cluster: Option<u32>,
    pub fat_type: Option<FatType>,
    pub root_entries: Option<u16>,
    pub media: Option<u8>,
    pub sectors_per_track: Option<u16>,
    pub heads: Option<u16>,
    pub drive_num: Option<u8>,
    pub volume_id: Option<u32>,
    pub volume_label: Option<[u8; 11]>,
    // force usage of Default trait by struct users
    _end: [u8;0],
}

const KB: u32 = 1024;
const MB: u32 = KB * 1024;
const GB: u32 = MB * 1024;

fn determine_fat_type(total_bytes: u64) -> FatType {
    if total_bytes < 4*MB as u64 {
        FatType::Fat12
    } else if total_bytes < 512*MB as u64 {
        FatType::Fat16
    } else {
        FatType::Fat32
    }
}

fn determine_bytes_per_cluster(total_bytes: u64, fat_type: FatType, bytes_per_sector: u16) -> u32 {
    let bytes_per_cluster = match fat_type {
        FatType::Fat12 => (total_bytes.next_power_of_two() / MB as u64) as u32 * 512,
        FatType::Fat16 => {
            if total_bytes <= 16 * MB as u64 {
                1 * KB
            } else if total_bytes <= 128 * MB as u64 {
                2 * KB
            } else {
                (total_bytes.next_power_of_two() / (64 * MB as u64)) as u32 * KB
            }
        },
        FatType::Fat32 => {
            if total_bytes <= 260 * MB as u64 {
                512
            } else if total_bytes <= 8 * GB as u64 {
                4 * KB
            } else {
                (total_bytes.next_power_of_two() / (2 * GB as u64)) as u32 * KB
            }
        },
    };
    const MAX_CLUSTER_SIZE: u32 = 32*KB;
    debug_assert!(bytes_per_cluster.is_power_of_two());
    cmp::min(cmp::max(bytes_per_cluster, bytes_per_sector as u32), MAX_CLUSTER_SIZE)
}

fn determine_sectors_per_fat(total_sectors: u32, reserved_sectors: u16, fats: u8, root_dir_sectors: u32,
        sectors_per_cluster: u8, fat_type: FatType) -> u32 {

    // TODO: use _fat_entries_per_sector
    // FIXME: this is for FAT16/32
    let tmp_val1 = total_sectors - (reserved_sectors as u32 + root_dir_sectors as u32);
    let mut tmp_val2 = (256 * sectors_per_cluster as u32) + fats as u32;
    if fat_type == FatType::Fat32 {
        tmp_val2 = tmp_val2 / 2;
    } else if fat_type == FatType::Fat12 {
        tmp_val2 = tmp_val2 / 3 * 4
    }
    let sectors_per_fat = (tmp_val1 + (tmp_val2 - 1)) / tmp_val2;

    // total_sectors = reserved_sectors + sectors_per_fat * fats + data_sectors
    // sectors_per_fat >= data_sectors / sectors_per_cluster / fat_entries_per_sector
    //
    // sectors_per_fat >= (total_sectors - reserved_sectors - sectors_per_fat * fats) / sectors_per_cluster / fat_entries_per_sector
    // sectors_per_fat + sectors_per_fat * fats / sectors_per_cluster / fat_entries_per_sector >= (total_sectors - reserved_sectors) / sectors_per_cluster / fat_entries_per_sector
    // sectors_per_fat * (1 + fats / sectors_per_cluster / fat_entries_per_sector) >= (total_sectors - reserved_sectors) / sectors_per_cluster / fat_entries_per_sector
    // sectors_per_fat >= (total_sectors - reserved_sectors) / sectors_per_cluster / fat_entries_per_sector / (1 + fats / sectors_per_cluster / fat_entries_per_sector)
    // fat_entries_per_sector = bytes_per_sector / bytes_per_fat_entry = fat16: 512/2
    sectors_per_fat
}

fn format_bpb(options: &FormatOptions) -> io::Result<(BiosParameterBlock, FatType)> {
    // TODO: maybe total_sectors could be optional?
    let bytes_per_sector = options.bytes_per_sector.unwrap_or(512);
    let total_sectors = options.total_sectors;
    let total_bytes = total_sectors as u64 * bytes_per_sector as u64;
    let fat_type = options.fat_type.unwrap_or_else(|| determine_fat_type(total_bytes));
    let bytes_per_cluster = options.bytes_per_cluster
        .unwrap_or_else(|| determine_bytes_per_cluster(total_bytes, fat_type, bytes_per_sector));
    let sectors_per_cluster = (bytes_per_cluster / bytes_per_sector as u32) as u8;

    // Note: most of implementations use 32 reserved sectors for FAT32 but it's wasting of space
    let reserved_sectors: u16 = if fat_type == FatType::Fat32 { 4 } else { 1 };

    let fats = 2u8;
    let is_fat32 = fat_type == FatType::Fat32;
    let root_entries = if is_fat32 { 0 } else { options.root_entries.unwrap_or(512) };
    let root_dir_bytes = root_entries as u32 * DIR_ENTRY_SIZE as u32;
    let root_dir_sectors = (root_dir_bytes + bytes_per_sector as u32 - 1) / bytes_per_sector as u32;

    if total_sectors <= reserved_sectors as u32 + root_dir_sectors as u32 + 16 {
        return Err(Error::new(ErrorKind::Other, "Volume is too small",));
    }

    //let fat_entries_per_sector = bytes_per_sector * 8 / fat_type.bits_per_fat_entry() as u16;
    let sectors_per_fat = determine_sectors_per_fat(total_sectors, reserved_sectors, fats, root_dir_sectors,
        sectors_per_cluster, fat_type);

    // drive_num should be 0 for floppy disks and 0x80 for hard disks - determine it using FAT type
    let drive_num = options.drive_num.unwrap_or_else(|| if fat_type == FatType::Fat12 { 0 } else { 0x80 });

    let reserved_0 = [0u8; 12];

    let mut volume_label = [0u8; 11];
    if let Some(volume_label_from_opts) = options.volume_label {
        volume_label.copy_from_slice(&volume_label_from_opts);
    } else {
        volume_label.copy_from_slice("NO NAME    ".as_bytes());
    }

    let mut fs_type_label = [0u8; 8];
    let fs_type_label_str = match fat_type {
        FatType::Fat12 => "FAT12   ",
        FatType::Fat16 => "FAT16   ",
        FatType::Fat32 => "FAT32   ",
    };
    fs_type_label.copy_from_slice(fs_type_label_str.as_bytes());

    let bpb = BiosParameterBlock {
        bytes_per_sector,
        sectors_per_cluster,
        reserved_sectors,
        fats,
        root_entries,
        total_sectors_16: if total_sectors < 0x10000 { total_sectors as u16 } else { 0 },
        media: options.media.unwrap_or(0xF8),
        sectors_per_fat_16: if is_fat32 { 0 } else { sectors_per_fat as u16 },
        sectors_per_track: options.sectors_per_track.unwrap_or(0x20),
        heads: options.heads.unwrap_or(0x40),
        hidden_sectors: 0,
        total_sectors_32: if total_sectors >= 0x10000 { total_sectors } else { 0 },
        // FAT32 fields start
        sectors_per_fat_32: if is_fat32 { sectors_per_fat } else { 0 },
        extended_flags: 0, // mirroring enabled
        fs_version: 0,
        root_dir_first_cluster: if is_fat32 { 2 } else { 0 },
        fs_info_sector: if is_fat32 { 1 } else { 0 },
        backup_boot_sector: if is_fat32 { 6 } else { 0 },
        reserved_0,
        // FAT32 fields end
        drive_num,
        reserved_1: 0,
        ext_sig: 0x29,
        volume_id: options.volume_id.unwrap_or(0x12345678),
        volume_label,
        fs_type_label,
    };

    if FatType::from_clusters(bpb.total_clusters()) != fat_type {
        return Err(Error::new(ErrorKind::Other, "Total number of clusters and FAT type does not match. Try other volume size"));
    }

    Ok((bpb, fat_type))
}

fn write_zeros<T: ReadWriteSeek>(mut disk: T, mut len: usize) -> io::Result<()> {
    const ZEROS: [u8; 512] = [0u8; 512];
    while len > 0 {
        let write_size = cmp::min(len, ZEROS.len());
        disk.write_all(&ZEROS[..write_size])?;
        len -= write_size;
    }
    Ok(())
}

fn write_zeros_until_end_of_sector<T: ReadWriteSeek>(mut disk: T, bytes_per_sector: u16) -> io::Result<()> {
    let pos = disk.seek(SeekFrom::Current(0))?;
    let total_bytes_to_write = bytes_per_sector as usize - (pos % bytes_per_sector as u64) as usize;
    if total_bytes_to_write != bytes_per_sector as usize {
        write_zeros(disk, total_bytes_to_write)?;
    }
    Ok(())
}

fn format_boot_sector(options: &FormatOptions) -> io::Result<(BootRecord, FatType)> {
    let mut boot: BootRecord = Default::default();
    let (bpb, fat_type) = format_bpb(options)?;
    boot.bpb = bpb;
    boot.oem_name.copy_from_slice("MSWIN4.1".as_bytes());
    // Boot code copied from FAT32 boot sector initialized by mkfs.fat
    boot.bootjmp = [0xEB, 0x58, 0x90];
    let boot_code: [u8; 129] = [
        0x0E, 0x1F, 0xBE, 0x77, 0x7C, 0xAC, 0x22, 0xC0, 0x74, 0x0B, 0x56, 0xB4, 0x0E, 0xBB, 0x07, 0x00,
        0xCD, 0x10, 0x5E, 0xEB, 0xF0, 0x32, 0xE4, 0xCD, 0x16, 0xCD, 0x19, 0xEB, 0xFE, 0x54, 0x68, 0x69,
        0x73, 0x20, 0x69, 0x73, 0x20, 0x6E, 0x6F, 0x74, 0x20, 0x61, 0x20, 0x62, 0x6F, 0x6F, 0x74, 0x61,
        0x62, 0x6C, 0x65, 0x20, 0x64, 0x69, 0x73, 0x6B, 0x2E, 0x20, 0x20, 0x50, 0x6C, 0x65, 0x61, 0x73,
        0x65, 0x20, 0x69, 0x6E, 0x73, 0x65, 0x72, 0x74, 0x20, 0x61, 0x20, 0x62, 0x6F, 0x6F, 0x74, 0x61,
        0x62, 0x6C, 0x65, 0x20, 0x66, 0x6C, 0x6F, 0x70, 0x70, 0x79, 0x20, 0x61, 0x6E, 0x64, 0x0D, 0x0A,
        0x70, 0x72, 0x65, 0x73, 0x73, 0x20, 0x61, 0x6E, 0x79, 0x20, 0x6B, 0x65, 0x79, 0x20, 0x74, 0x6F,
        0x20, 0x74, 0x72, 0x79, 0x20, 0x61, 0x67, 0x61, 0x69, 0x6E, 0x20, 0x2E, 0x2E, 0x2E, 0x20, 0x0D,
        0x0A];
    boot.boot_code[..boot_code.len()].copy_from_slice(&boot_code);
    boot.boot_sig = [0x55, 0xAA];

    // fix offsets in bootjmp and boot code for non-FAT32 filesystems (bootcode is on a different offset)
    if fat_type != FatType::Fat32 {
        // offset of boot code
        let boot_code_offset = 0x36 + 8;
        boot.bootjmp[1] = (boot_code_offset - 2) as u8;
        // offset of message
        const MESSAGE_OFFSET: u32 = 29;
        let message_offset_in_sector = boot_code_offset + MESSAGE_OFFSET + 0x7c00;
        boot.boot_code[3] = (message_offset_in_sector & 0xff) as u8;
        boot.boot_code[4] = (message_offset_in_sector >> 8) as u8;
    }

    Ok((boot, fat_type))
}

// alternative names: create_filesystem, init_filesystem, prepare_fs
pub fn format_volume<T: ReadWriteSeek>(mut disk: T, options: FormatOptions) -> io::Result<()> {
    let (boot, fat_type) = format_boot_sector(&options)?;
    boot.serialize(&mut disk)?;
    let bytes_per_sector = boot.bpb.bytes_per_sector;
    write_zeros_until_end_of_sector(&mut disk, bytes_per_sector)?;

    if boot.bpb.is_fat32() {
        // FSInfo sector
        let fs_info_sector = FsInfoSector {
            free_cluster_count: None,
            next_free_cluster: None,
            dirty: false,
        };
        disk.seek(SeekFrom::Start(boot.bpb.fs_info_sector as u64 * bytes_per_sector as u64))?;
        fs_info_sector.serialize(&mut disk)?;
        write_zeros_until_end_of_sector(&mut disk, bytes_per_sector)?;

        // backup boot sector
        disk.seek(SeekFrom::Start(boot.bpb.backup_boot_sector as u64 * bytes_per_sector as u64))?;
        boot.serialize(&mut disk)?;
        write_zeros_until_end_of_sector(&mut disk, bytes_per_sector)?;
    }

    // FATs
    let sectors_per_fat: u32 = boot.bpb.sectors_per_fat();
    let bytes_per_fat: u32 = sectors_per_fat * bytes_per_sector as u32;
    let reserved_sectors = boot.bpb.reserved_sectors;
    let fat_pos = reserved_sectors as u64 * bytes_per_sector as u64;
    disk.seek(SeekFrom::Start(fat_pos))?;
    write_zeros(&mut disk, bytes_per_fat as usize * boot.bpb.fats as usize)?;
    {
        let mut fat_slice = fat_slice(&mut disk, &boot.bpb);
        format_fat(&mut fat_slice, fat_type, boot.bpb.media, bytes_per_fat, boot.bpb.total_clusters())?;
    }

    // Root directory
    let root_dir_pos = fat_pos + bytes_per_fat as u64 * boot.bpb.fats as u64;
    disk.seek(SeekFrom::Start(root_dir_pos))?;
    let root_dir_sectors: u32 = boot.bpb.root_dir_sectors();
    write_zeros(&mut disk, root_dir_sectors as usize * bytes_per_sector as usize)?;
    if fat_type == FatType::Fat32 {
        let root_dir_first_cluster = {
            let mut fat_slice = fat_slice(&mut disk, &boot.bpb);
            alloc_cluster(&mut fat_slice, fat_type, None, None, 1)?
        };
        assert!(root_dir_first_cluster == boot.bpb.root_dir_first_cluster);
        let first_data_sector = reserved_sectors as u32 + sectors_per_fat + root_dir_sectors;
        let sectors_per_cluster = boot.bpb.sectors_per_cluster;
        let root_dir_first_sector =
            ((root_dir_first_cluster - RESERVED_FAT_ENTRIES) * sectors_per_cluster as u32) + first_data_sector;
        let root_dir_pos = root_dir_first_sector as u64 * bytes_per_sector as u64;
        disk.seek(SeekFrom::Start(root_dir_pos))?;
        write_zeros(&mut disk, sectors_per_cluster as usize * bytes_per_sector as usize)?;
    }

    // TODO: create volume label dir entry if volume label is set

    disk.seek(SeekFrom::Start(0))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_determine_fat_type() {
        assert_eq!(determine_fat_type(3 * MB as u64), FatType::Fat12);
        assert_eq!(determine_fat_type(4 * MB as u64), FatType::Fat16);
        assert_eq!(determine_fat_type(511 * MB as u64), FatType::Fat16);
        assert_eq!(determine_fat_type(512 * MB as u64), FatType::Fat32);
    }

    #[test]
    fn test_determine_bytes_per_cluster_fat12() {
        assert_eq!(determine_bytes_per_cluster(1 * MB as u64 + 0, FatType::Fat12, 512), 512);
        assert_eq!(determine_bytes_per_cluster(1 * MB as u64 + 1, FatType::Fat12, 512), 1024);
        assert_eq!(determine_bytes_per_cluster(1 * MB as u64, FatType::Fat12, 4096), 4096);
    }

    #[test]
    fn test_determine_bytes_per_cluster_fat16() {
        assert_eq!(determine_bytes_per_cluster(1 * MB as u64, FatType::Fat16, 512), 1 * KB);
        assert_eq!(determine_bytes_per_cluster(1 * MB as u64, FatType::Fat16, 4 * KB as u16), 4 * KB);
        assert_eq!(determine_bytes_per_cluster(16 * MB as u64 + 0, FatType::Fat16, 512), 1 * KB);
        assert_eq!(determine_bytes_per_cluster(16 * MB as u64 + 1, FatType::Fat16, 512), 2 * KB);
        assert_eq!(determine_bytes_per_cluster(128 * MB as u64 + 0, FatType::Fat16, 512), 2 * KB);
        assert_eq!(determine_bytes_per_cluster(128 * MB as u64 + 1, FatType::Fat16, 512), 4 * KB);
        assert_eq!(determine_bytes_per_cluster(256 * MB as u64 + 0, FatType::Fat16, 512), 4 * KB);
        assert_eq!(determine_bytes_per_cluster(256 * MB as u64 + 1, FatType::Fat16, 512), 8 * KB);
        assert_eq!(determine_bytes_per_cluster(512 * MB as u64 + 0, FatType::Fat16, 512), 8 * KB);
        assert_eq!(determine_bytes_per_cluster(512 * MB as u64 + 1, FatType::Fat16, 512), 16 * KB);
        assert_eq!(determine_bytes_per_cluster(1024 * MB as u64 + 0, FatType::Fat16, 512), 16 * KB);
        assert_eq!(determine_bytes_per_cluster(1024 * MB as u64 + 1, FatType::Fat16, 512), 32 * KB);
        assert_eq!(determine_bytes_per_cluster(99999 * MB as u64, FatType::Fat16, 512), 32 * KB);
    }

    #[test]
    fn test_determine_bytes_per_cluster_fat32() {
        assert_eq!(determine_bytes_per_cluster(260 * MB as u64, FatType::Fat32, 512), 512);
        assert_eq!(determine_bytes_per_cluster(260 * MB as u64, FatType::Fat32, 4 * KB as u16), 4 * KB);
        assert_eq!(determine_bytes_per_cluster(260 * MB as u64 + 1, FatType::Fat32, 512), 4 * KB);
        assert_eq!(determine_bytes_per_cluster(8 * GB as u64, FatType::Fat32, 512), 4 * KB);
        assert_eq!(determine_bytes_per_cluster(8 * GB as u64 + 1, FatType::Fat32, 512), 8 * KB);
        assert_eq!(determine_bytes_per_cluster(16 * GB as u64 + 0, FatType::Fat32, 512), 8 * KB);
        assert_eq!(determine_bytes_per_cluster(16 * GB as u64 + 1, FatType::Fat32, 512), 16 * KB);
        assert_eq!(determine_bytes_per_cluster(32 * GB as u64, FatType::Fat32, 512), 16 * KB);
        assert_eq!(determine_bytes_per_cluster(32 * GB as u64 + 1, FatType::Fat32, 512), 32 * KB);
        assert_eq!(determine_bytes_per_cluster(999 * GB as u64, FatType::Fat32, 512), 32 * KB);
    }

    #[test]
    fn test_determine_sectors_per_fat() {
        assert_eq!(determine_sectors_per_fat(1 * MB / 512, 1, 2, 32, 1, FatType::Fat12), 6);
    }
}
