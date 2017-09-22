extern crate byteorder;

use std::fs::File;
use std::io::prelude::*;
use std::io::BufReader;
use std::io;
use std::str;
use std::io::{Error, ErrorKind};
use byteorder::{LittleEndian, ReadBytesExt};

// FAT implementation based on:
//   http://wiki.osdev.org/FAT
//   https://www.win.tue.nl/~aeb/linux/fs/fat/fat-1.html

#[derive(Debug, Copy, Clone)]
enum FatType {
    Fat12, Fat16, Fat32, ExFat
}

#[derive(Debug)]
struct FatFileSystem {
    fat_type: FatType,
    root_cluster: u32,
    volume_id: u32,
    volume_label: String,
    first_data_sector: u32,
    first_fat_sector: u32,
}

impl FatFileSystem {
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
    
    pub fn new(rdr: &mut BufRead) -> io::Result<FatFileSystem> {
        let mut bootjmp = [0; 3];
        rdr.read(&mut bootjmp)?;
        let mut oem_name: [u8; 8] = [0; 8];
        rdr.read(&mut oem_name)?;
        let bytes_per_sector = rdr.read_u16::<LittleEndian>()? as u32;
        let sectors_per_cluster = rdr.read_u8()? as u32;
        let reserved_sector_count = rdr.read_u16::<LittleEndian>()? as u32;
        let table_count = rdr.read_u8()? as u32;
        let root_entry_count = rdr.read_u16::<LittleEndian>()? as u32;
        let total_sectors_16 = rdr.read_u16::<LittleEndian>()? as u32;
        rdr.read_u8()?; // media_type
        let table_size_16 = rdr.read_u16::<LittleEndian>()? as u32;
        rdr.read_u16::<LittleEndian>()?; // sectors_per_track
        rdr.read_u16::<LittleEndian>()?; // head_side_count
        rdr.read_u32::<LittleEndian>()?; // hidden_sector_count
        let total_sectors_32 = rdr.read_u32::<LittleEndian>()? as u32;
        
        let (fat_size, fat_info, root_cluster, volume_id);
        let mut volume_label = [0; 11];
        let mut fat_type_label = [0; 8];
        
        if table_size_16 == 0 {
            let table_size_32 = rdr.read_u32::<LittleEndian>()? as u32;
            rdr.read_u16::<LittleEndian>()?; // extended_flags
            rdr.read_u16::<LittleEndian>()?; // fat_version
            root_cluster = rdr.read_u32::<LittleEndian>()?;
            fat_info = rdr.read_u16::<LittleEndian>()? as u32;
            rdr.read_u16::<LittleEndian>()?; // backup_bs_sector
            let mut reserved_0 = [0; 12];
            rdr.read(&mut reserved_0)?;
            rdr.read_u8()?; // drive_num
            rdr.read_u8()?; // reserved_1
            rdr.read_u8()?; // ext_sig (0x29)
            volume_id = rdr.read_u32::<LittleEndian>()?;
            rdr.read(&mut volume_label)?;
            rdr.read(&mut fat_type_label)?;
            let mut boot_code = [0; 420];
            rdr.read(&mut boot_code)?;
            
            fat_size = table_size_32;
        } else {
            rdr.read_u8()?; // drive_num
            rdr.read_u8()?; // reserved1
            rdr.read_u8()?; // ext_sig (0x29)
            volume_id = rdr.read_u32::<LittleEndian>()?;
            rdr.read(&mut volume_label)?;
            rdr.read(&mut fat_type_label)?;
            let mut boot_code = [0; 448];
            rdr.read(&mut boot_code)?;
            
            fat_size = table_size_16;
            fat_info = 0;
            root_cluster = 0;
        }
        let mut boot_sig = [0; 2];
        rdr.read(&mut boot_sig)?;
        if boot_sig != [0x55, 0xAA] {
            return Err(Error::new(ErrorKind::Other, "invalid signature"));
        }
        
        let total_sectors = if total_sectors_16 == 0 { total_sectors_32 } else { total_sectors_16 };
        let root_dir_sectors = ((root_entry_count * 32) + (bytes_per_sector - 1)) / (bytes_per_sector);
        let first_data_sector = reserved_sector_count + (table_count * fat_size) + root_dir_sectors;
        let first_fat_sector = reserved_sector_count;
        let data_sectors = total_sectors - (reserved_sector_count + (table_count * fat_size) + root_dir_sectors);
        let total_clusters = data_sectors / sectors_per_cluster;
        let fat_type = FatFileSystem::fat_type_from_clusters(total_clusters);
        
        let fs = FatFileSystem {
            fat_type: fat_type,
            root_cluster: root_cluster,
            volume_id: volume_id,
            volume_label: str::from_utf8(&volume_label).unwrap().to_string(),
            first_data_sector: first_data_sector,
            first_fat_sector: first_fat_sector,
        };
        
        println!("fat_type {:?}", fat_type);
        println!("fat_info {}", fat_info);
        println!("root_cluster {}", root_cluster);
        println!("volume_id {}", volume_id);
        println!("oem_name {}", str::from_utf8(&oem_name).unwrap());
        println!("volume_label {}", str::from_utf8(&volume_label).unwrap());
        println!("fat_type_label {}", str::from_utf8(&fat_type_label).unwrap());
        Ok(fs)
    }
}

fn fat_test() -> io::Result<()> {
    let file = File::open("resources/floppy.img")?;
    let mut buf_rdr = BufReader::new(file);
    FatFileSystem::new(&mut buf_rdr)?;
    Ok(())
}

fn main() {
    println!("Hello, world!");
    fat_test().unwrap();
}
