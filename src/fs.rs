#[cfg(all(not(feature = "std"), feature = "alloc"))]
use alloc::String;
use core::cell::RefCell;
use core::char;
use core::cmp;
use core::iter::FromIterator;
use io;
use io::prelude::*;
use io::{Error, ErrorKind, SeekFrom};

use byteorder::LittleEndian;
use byteorder_ext::{ReadBytesExt, WriteBytesExt};

use dir::{Dir, DirRawStream};
use dir_entry::DIR_ENTRY_SIZE;
use file::File;
use table::{alloc_cluster, count_free_clusters, read_fat_flags, ClusterIterator};

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
}

/// A sum of `Read` and `Seek` traits.
pub trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

/// A sum of `Read`, `Write` and `Seek` traits.
pub trait ReadWriteSeek: Read + Write + Seek {}
impl<T: Read + Write + Seek> ReadWriteSeek for T {}

#[allow(dead_code)]
#[derive(Default, Debug, Clone)]
struct BiosParameterBlock {
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
        if bpb.ext_sig != 0x29 {
            // fields after ext_sig are not used - clean them
            bpb.volume_id = 0;
            bpb.volume_label = [0; 11];
            bpb.fs_type_label = [0; 8];
        }
        Ok(bpb)
    }

    fn mirroring_enabled(&self) -> bool {
        self.extended_flags & 0x80 == 0
    }

    fn active_fat(&self) -> u16 {
        self.extended_flags & 0x0F
    }

    fn status_flags(&self) -> FsStatusFlags {
        FsStatusFlags {
            dirty: self.reserved_1 & 1 != 0,
            io_error: self.reserved_1 & 2 != 0,
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

        if boot.bpb.sectors_per_fat_16 == 0 {
            rdr.read_exact(&mut boot.boot_code[0..420])?;
        } else {
            rdr.read_exact(&mut boot.boot_code[0..448])?;
        }
        rdr.read_exact(&mut boot.boot_sig)?;
        Ok(boot)
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
            n => Some(n),
        };
        let next_free_cluster = match rdr.read_u32::<LittleEndian>()? {
            0xFFFFFFFF => None,
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
//#[derive(Copy, Clone, Debug)]
pub struct FsOptions {
    pub(crate) update_accessed_date: bool,
    pub(crate) oem_cp_converter: &'static OemCpConverter,
}

impl FsOptions {
    /// Creates a `FsOptions` struct with default options.
    pub fn new() -> Self {
        FsOptions {
            update_accessed_date: false,
            oem_cp_converter: &LOSSY_OEM_CP_CONVERTER,
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
            if boot.boot_sig != [0x55, 0xAA] {
                return Err(Error::new(ErrorKind::Other, "Invalid boot sector signature"));
            }
            boot.bpb
        };

        if bpb.fs_version != 0 {
            return Err(Error::new(ErrorKind::Other, "Unknown FS version"));
        }

        let total_sectors = if bpb.total_sectors_16 == 0 {
            bpb.total_sectors_32
        } else {
            bpb.total_sectors_16 as u32
        };
        let sectors_per_fat = if bpb.sectors_per_fat_16 == 0 {
            bpb.sectors_per_fat_32
        } else {
            bpb.sectors_per_fat_16 as u32
        };
        let root_dir_bytes = bpb.root_entries as u32 * DIR_ENTRY_SIZE as u32;
        let root_dir_sectors = (root_dir_bytes + (bpb.bytes_per_sector as u32 - 1)) / bpb.bytes_per_sector as u32;
        let first_data_sector = bpb.reserved_sectors as u32 + (bpb.fats as u32 * sectors_per_fat) + root_dir_sectors;
        let fat_sectors = bpb.fats as u32 * sectors_per_fat;
        let data_sectors = total_sectors - (bpb.reserved_sectors as u32 + fat_sectors + root_dir_sectors as u32);
        let total_clusters = data_sectors / bpb.sectors_per_cluster as u32;
        let fat_type = FatType::from_clusters(total_clusters);

        // read FSInfo sector if this is FAT32
        let mut fs_info = if fat_type == FatType::Fat32 {
            disk.seek(SeekFrom::Start(bpb.fs_info_sector as u64 * 512))?;
            FsInfoSector::deserialize(&mut disk)?
        } else {
            FsInfoSector::default()
        };

        // if dirty flag is set completly ignore free_cluster_count in FSInfo
        if bpb.status_flags().dirty {
            fs_info.free_cluster_count = None;
        }

        // return FileSystem struct
        Ok(FileSystem {
            disk: RefCell::new(disk),
            options,
            fat_type,
            bpb,
            first_data_sector,
            root_dir_sectors,
            total_clusters,
            fs_info: RefCell::new(fs_info),
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
        let full_label_slice = &self.bpb.volume_label;
        let len = full_label_slice.iter().rposition(|b| *b != 0x20).map(|p| p + 1).unwrap_or(0);
        &full_label_slice[..len]
    }

    /// Returns a root directory object allowing for futher penetration of a filesystem structure.
    pub fn root_dir<'b>(&'b self) -> Dir<'b, T> {
        let root_rdr = {
            match self.fat_type {
                FatType::Fat12 | FatType::Fat16 => DirRawStream::Root(DiskSlice::from_sectors(
                    self.first_data_sector - self.root_dir_sectors,
                    self.root_dir_sectors,
                    1,
                    self,
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

    fn fat_slice<'b>(&'b self) -> DiskSlice<'b, T> {
        let sectors_per_fat = if self.bpb.sectors_per_fat_16 == 0 {
            self.bpb.sectors_per_fat_32
        } else {
            self.bpb.sectors_per_fat_16 as u32
        };
        let mirroring_enabled = self.bpb.mirroring_enabled();
        let (fat_first_sector, mirrors) = if mirroring_enabled {
            (self.bpb.reserved_sectors as u32, self.bpb.fats)
        } else {
            let active_fat = self.bpb.active_fat() as u32;
            let fat_first_sector = (self.bpb.reserved_sectors as u32) + active_fat * sectors_per_fat;
            (fat_first_sector, 1)
        };
        DiskSlice::from_sectors(fat_first_sector, sectors_per_fat, mirrors, self)
    }

    pub(crate) fn cluster_iter<'b>(&'b self, cluster: u32) -> ClusterIterator<DiskSlice<'b, T>> {
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
        Ok(())
    }

    fn flush_fs_info(&self) -> io::Result<()> {
        let mut fs_info = self.fs_info.borrow_mut();
        if self.fat_type == FatType::Fat32 && fs_info.dirty {
            let mut disk = self.disk.borrow_mut();
            disk.seek(SeekFrom::Start(self.bpb.fs_info_sector as u64 * 512))?;
            fs_info.serialize(&mut *disk)?;
            fs_info.dirty = false;
        }
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

pub(crate) struct DiskSlice<'a, T: ReadWriteSeek + 'a> {
    begin: u64,
    size: u64,
    offset: u64,
    mirrors: u8,
    fs: &'a FileSystem<T>,
}

impl<'a, T: ReadWriteSeek> DiskSlice<'a, T> {
    pub(crate) fn new(begin: u64, size: u64, mirrors: u8, fs: &'a FileSystem<T>) -> Self {
        DiskSlice {
            begin,
            size,
            mirrors,
            fs,
            offset: 0,
        }
    }

    pub(crate) fn from_sectors(first_sector: u32, sector_count: u32, mirrors: u8, fs: &'a FileSystem<T>) -> Self {
        let bytes_per_sector = fs.bpb.bytes_per_sector as u64;
        Self::new(
            first_sector as u64 * bytes_per_sector,
            sector_count as u64 * bytes_per_sector,
            mirrors,
            fs,
        )
    }

    pub(crate) fn abs_pos(&self) -> u64 {
        self.begin + self.offset
    }
}

// Note: derive cannot be used because of invalid bounds. See: https://github.com/rust-lang/rust/issues/26925
impl<'a, T: ReadWriteSeek> Clone for DiskSlice<'a, T> {
    fn clone(&self) -> Self {
        DiskSlice {
            begin: self.begin,
            size: self.size,
            offset: self.offset,
            mirrors: self.mirrors,
            fs: self.fs,
        }
    }
}

impl<'a, T: ReadWriteSeek> Read for DiskSlice<'a, T> {
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

impl<'a, T: ReadWriteSeek> Write for DiskSlice<'a, T> {
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

impl<'a, T: ReadWriteSeek> Seek for DiskSlice<'a, T> {
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
pub trait OemCpConverter {
    fn decode(&self, oem_char: u8) -> char;
    fn encode(&self, uni_char: char) -> Option<u8>;
}

#[derive(Clone)]
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
