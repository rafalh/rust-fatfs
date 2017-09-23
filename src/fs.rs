use std::cell::RefCell;
use std::cmp;
use std::io::prelude::*;
use std::io::{Error, ErrorKind};
use std::io::SeekFrom;
use std::io;
use std::str;
use std::rc::Rc;
use byteorder::{LittleEndian, ReadBytesExt};

use file::FatFile;
use dir::FatDir;
use table::{FatTable, FatTable12, FatTable16, FatTable32};

// FAT implementation based on:
//   http://wiki.osdev.org/FAT
//   https://www.win.tue.nl/~aeb/linux/fs/fat/fat-1.html

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum FatType {
    Fat12, Fat16, Fat32, ExFat
}

pub trait ReadSeek: Read + Seek {}
impl<T> ReadSeek for T where T: Read + Seek {}

#[allow(dead_code)]
pub(crate) struct FatSharedState {
    pub rdr: Box<ReadSeek>,
    pub fat_type: FatType,
    pub boot: FatBootRecord,
    pub first_fat_sector: u32,
    pub first_data_sector: u32,
    pub root_dir_sectors: u32,
    pub table: Box<FatTable>,
}

impl FatSharedState {
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
}

pub(crate) type FatSharedStateRef = Rc<RefCell<FatSharedState>>;

#[allow(dead_code)]
pub struct FatFileSystem {
    pub(crate) state: FatSharedStateRef,
}

#[allow(dead_code)]
#[derive(Default, Debug)]
pub(crate) struct FatBiosParameterBlock {
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    reserved_sector_count: u16,
    table_count: u8,
    root_entry_count: u16,
    total_sectors_16: u16,
    media_type: u8,
    table_size_16: u16,
    sectors_per_track: u16,
    head_side_count: u16,
    hidden_sector_count: u32,
    total_sectors_32: u32,
    
    // Extended BIOS Parameter Block
    table_size_32: u32,
    extended_flags: u16,
    fat_version: u16,
    root_cluster: u32,
    fat_info: u16,
    backup_bs_sector: u16,
    reserved_0: [u8; 12],
    drive_num: u8,
    reserved_1: u8,
    ext_sig: u8,
    volume_id: u32,
    volume_label: [u8; 11],
    fat_type_label: [u8; 8],
}

#[allow(dead_code)]
pub(crate) struct FatBootRecord {
    bootjmp: [u8; 3],
    oem_name: [u8; 8],
    bpb: FatBiosParameterBlock,
    boot_code: [u8; 448],
    boot_sig: [u8; 2],
}

impl Default for FatBootRecord {
    fn default() -> FatBootRecord { 
        FatBootRecord {
            bootjmp: Default::default(),
            oem_name: Default::default(),
            bpb: Default::default(),
            boot_code: [0; 448],
            boot_sig: Default::default(),
        }
    }
}

impl FatFileSystem {
    
    //pub fn new<T: ReadSeek + 'static>(rdr: T) -> io::Result<FatFileSystem> {
    pub fn new<T: ReadSeek + 'static>(mut rdr: Box<T>) -> io::Result<FatFileSystem> {
        let boot = Self::read_boot_record(&mut *rdr)?;
        if boot.boot_sig != [0x55, 0xAA] {
            return Err(Error::new(ErrorKind::Other, "invalid signature"));
        }
        
        let total_sectors = if boot.bpb.total_sectors_16 == 0 { boot.bpb.total_sectors_32 } else { boot.bpb.total_sectors_16 as u32 };
        let table_size = if boot.bpb.table_size_16 == 0 { boot.bpb.table_size_32 } else { boot.bpb.table_size_16 as u32 };
        let root_dir_sectors = ((boot.bpb.root_entry_count * 32) + (boot.bpb.bytes_per_sector - 1)) / (boot.bpb.bytes_per_sector);
        let first_data_sector = boot.bpb.reserved_sector_count as u32 + (boot.bpb.table_count as u32 * table_size) + root_dir_sectors as u32;
        let first_fat_sector = boot.bpb.reserved_sector_count as u32;
        let data_sectors = total_sectors - (boot.bpb.reserved_sector_count as u32 + (boot.bpb.table_count as u32 * table_size) + root_dir_sectors as u32);
        let total_clusters = data_sectors / boot.bpb.sectors_per_cluster as u32;
        let fat_type = Self::fat_type_from_clusters(total_clusters);
        
        {
            let oem_name_str = str::from_utf8(&boot.oem_name).unwrap().trim_right();
            let volume_label_str = str::from_utf8(&boot.bpb.volume_label).unwrap().trim_right();
            let fat_type_label_str = str::from_utf8(&boot.bpb.fat_type_label).unwrap().trim_right();
            
            println!("fat_type {:?}", fat_type);
            println!("volume_id {}", boot.bpb.volume_id);
            println!("oem_name {}", oem_name_str);
            println!("volume_label {}", volume_label_str);
            println!("fat_type_label {}", fat_type_label_str);
        }
        
        // FIXME: other versions
        let table_size_bytes = table_size * boot.bpb.bytes_per_sector as u32;
        let table: Box<FatTable> = match fat_type {
            FatType::Fat12 => Box::new(FatTable12::read(&mut rdr, table_size_bytes as usize)?),
            FatType::Fat16 => Box::new(FatTable16::read(&mut rdr, table_size_bytes as usize)?),
            FatType::Fat32 => Box::new(FatTable32::read(&mut rdr, table_size_bytes as usize)?),
            _ => panic!("TODO: exfat")
        };
        
        let rdr_box = Box::new(rdr);
        let state = FatSharedState {
            rdr: rdr_box,
            fat_type: fat_type,
            boot: boot,
            first_data_sector: first_data_sector,
            first_fat_sector: first_fat_sector,
            root_dir_sectors: root_dir_sectors as u32,
            table: table,
        };
        
        Ok(FatFileSystem {
            state: Rc::new(RefCell::new(state)),
        })
    }
    
    fn read_bpb(rdr: &mut Read) -> io::Result<FatBiosParameterBlock> {
        let mut bpb: FatBiosParameterBlock = Default::default();
        bpb.bytes_per_sector = rdr.read_u16::<LittleEndian>()?;
        bpb.sectors_per_cluster = rdr.read_u8()?;
        bpb.reserved_sector_count = rdr.read_u16::<LittleEndian>()?;
        bpb.table_count = rdr.read_u8()?;
        bpb.root_entry_count = rdr.read_u16::<LittleEndian>()? ;
        bpb.total_sectors_16 = rdr.read_u16::<LittleEndian>()?;
        bpb.media_type = rdr.read_u8()?;
        bpb.table_size_16 = rdr.read_u16::<LittleEndian>()?;
        bpb.sectors_per_track = rdr.read_u16::<LittleEndian>()?;
        bpb.head_side_count = rdr.read_u16::<LittleEndian>()?;
        bpb.hidden_sector_count = rdr.read_u32::<LittleEndian>()?; // hidden_sector_count
        bpb.total_sectors_32 = rdr.read_u32::<LittleEndian>()?;
        
        if bpb.table_size_16 == 0 {
            bpb.table_size_32 = rdr.read_u32::<LittleEndian>()?;
            bpb.extended_flags = rdr.read_u16::<LittleEndian>()?;
            bpb.fat_version = rdr.read_u16::<LittleEndian>()?;
            bpb.root_cluster = rdr.read_u32::<LittleEndian>()?;
            bpb.fat_info = rdr.read_u16::<LittleEndian>()?;
            bpb.backup_bs_sector = rdr.read_u16::<LittleEndian>()?;
            rdr.read(&mut bpb.reserved_0)?;
            bpb.drive_num = rdr.read_u8()?;
            bpb.reserved_1 = rdr.read_u8()?;
            bpb.ext_sig = rdr.read_u8()?; // 0x29
            bpb.volume_id = rdr.read_u32::<LittleEndian>()?;
            rdr.read(&mut bpb.volume_label)?;
            rdr.read(&mut bpb.fat_type_label)?;
            //bpb.boot_code = Vec::with_capacity(420);
            //rdr.read_exact(bpb.boot_code.as_mut_slice())?;
        } else {
            bpb.drive_num = rdr.read_u8()?;
            bpb.reserved_1 = rdr.read_u8()?;
            bpb.ext_sig = rdr.read_u8()?; // 0x29
            bpb.volume_id = rdr.read_u32::<LittleEndian>()?;
            rdr.read(&mut bpb.volume_label)?;
            rdr.read(&mut bpb.fat_type_label)?;
            //bpb.boot_code = Vec::with_capacity(448);
            //rdr.read_exact(bpb.boot_code.as_mut_slice())?;
        }
        Ok(bpb)
    }
    
    fn fat_type_from_clusters(total_clusters: u32) -> FatType {
        if total_clusters < 4085 {
            FatType::Fat12
        } else if total_clusters < 65525 {
            FatType::Fat16
        } else if total_clusters < 268435445 {
            FatType::Fat32
        } else {
            FatType::ExFat
        }
    }
    
    fn read_boot_record(rdr: &mut Read) -> io::Result<FatBootRecord> {
        let mut boot: FatBootRecord = Default::default();
        rdr.read(&mut boot.bootjmp)?;
        rdr.read(&mut boot.oem_name)?;
        boot.bpb = Self::read_bpb(rdr)?;
        
        if boot.bpb.table_size_16 == 0 {
            rdr.read_exact(&mut boot.boot_code[0..420])?;
        } else {
            rdr.read_exact(&mut boot.boot_code[0..448])?;
        }
        rdr.read(&mut boot.boot_sig)?;
        Ok(boot)
    }
    
    pub fn root_dir(&mut self) -> FatDir {
        let state = self.state.borrow();
        let root_rdr: Box<Read> = match state.fat_type {
            FatType::Fat12 | FatType::Fat16 => Box::new(FatSlice::from_sectors(
                state.first_data_sector - state.root_dir_sectors, state.root_dir_sectors, self.state.clone())),
            _ => Box::new(FatFile::new(state.boot.bpb.root_cluster, state.root_dir_sectors, self.state.clone()))
        };
        FatDir::new(root_rdr, self.state.clone())
    }
}

struct FatSlice {
    begin: u64,
    size: u64,
    offset: u64,
    state: FatSharedStateRef,
}

impl FatSlice {
    pub(crate) fn new(begin: u64, size: u64, state: FatSharedStateRef) -> FatSlice {
        FatSlice { begin, size, state, offset: 0 }
    }
    
    pub(crate) fn from_sectors(first_sector: u32, sectors_count: u32, state: FatSharedStateRef) -> FatSlice {
        let bytes_per_sector = state.borrow().boot.bpb.bytes_per_sector as u64;
        Self::new(first_sector as u64 * bytes_per_sector, sectors_count as u64 * bytes_per_sector, state)
    }
}

impl Read for FatSlice {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let offset = self.begin + self.offset;
        let read_size = cmp::min((self.size - self.offset) as usize, buf.len());
        let mut state = self.state.borrow_mut();
        state.rdr.seek(SeekFrom::Start(offset))?;
        let size = state.rdr.read(&mut buf[..read_size])?;
        self.offset += size as u64;
        Ok(size)
    }
}
