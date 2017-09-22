use std::io::prelude::*;
use std::io;
use std::io::Cursor;
use std::str;
use byteorder::{LittleEndian, ReadBytesExt};
use chrono::{DateTime, Date, TimeZone, Local};

use fs::FatFileSystem;
use file::FatFile;

bitflags! {
    pub struct FatFileAttributes: u8 {
        const READ_ONLY  = 0x01;
        const HIDDEN     = 0x02;
        const SYSTEM     = 0x04;
        const VOLUME_ID  = 0x08;
        const DIRECTORY  = 0x10;
        const ARCHIVE    = 0x20;
        const LFN        = Self::READ_ONLY.bits | Self::HIDDEN.bits
                         | Self::SYSTEM.bits | Self::VOLUME_ID.bits;
    }
}

#[allow(dead_code)]
pub struct FatDirEntry {
    name: [u8; 11],
    attrs: FatFileAttributes,
    reserved_0: u8,
    create_time_0: u8,
    create_time_1: u16,
    create_date: u16,
    access_date: u16,
    first_cluster_hi: u16,
    modify_time: u16,
    modify_date: u16,
    first_cluster_lo: u16,
    size: u32,
}

fn convert_date(dos_date: u16) -> Date<Local> {
    let (year, month, day) = ((dos_date >> 9) + 1980, (dos_date >> 5) & 0xF, dos_date & 0x1F);
    Local.ymd(year as i32, month as u32, day as u32)
}

fn convert_date_time(dos_date: u16, dos_time: u16) -> DateTime<Local> {
    let (hour, min, sec) = (dos_time >> 11, (dos_time >> 5) & 0x3F, (dos_time & 0x1F) * 2);
    convert_date(dos_date).and_hms(hour as u32, min as u32, sec as u32)
}

impl FatDirEntry {
    
    pub fn get_name(&self) -> String {
        str::from_utf8(&self.name).unwrap().trim_right().to_string()
    }
    
    pub fn get_attrs(&self) -> FatFileAttributes {
        self.attrs
    }
    
    pub fn get_cluster(&self) -> u32 {
        ((self.first_cluster_hi as u32) << 16) | self.first_cluster_lo as u32
    }
    
    pub fn get_file<T: Read+Seek>(&self, fs: &FatFileSystem<T>) -> FatFile {
        FatFile::new(fs.sector_from_cluster(self.get_cluster()), self.size)
    }
    
    pub fn get_size(&self) -> u32 {
        self.size
    }
    
    pub fn get_create_time(&self) -> DateTime<Local> {
        convert_date_time(self.create_date, self.create_time_1)
    }
    
    pub fn get_access_date(&self) -> Date<Local> {
        convert_date(self.access_date)
    }
    
    pub fn get_modify_time(&self) -> DateTime<Local> {
        convert_date_time(self.modify_date, self.modify_time)
    }
}

impl<T: Read+Seek> FatFileSystem<T> {
    pub fn read_dir(&mut self, dir: &mut FatFile) -> io::Result<Vec<FatDirEntry>> {
        
        let mut cur = Cursor::new(vec![0; 512]);
        self.read(dir, cur.get_mut())?;
        
        let mut entries = Vec::new();
        loop {
            let entry = read_dir_entry(&mut cur)?;
            if entry.name[0] == 0 {
                break; // end of dir
            }
            if entry.name[0] == 0xE5 {
                continue; // deleted
            }
            entries.push(entry);
        }
        Ok(entries)
    }
}

fn read_dir_entry(rdr: &mut Read) -> io::Result<FatDirEntry> {
    let mut name = [0; 11];
    rdr.read(&mut name)?;
    Ok(FatDirEntry {
        name:             name,
        attrs:            FatFileAttributes::from_bits(rdr.read_u8()?).unwrap(),
        reserved_0:       rdr.read_u8()?,
        create_time_0:    rdr.read_u8()?,
        create_time_1:    rdr.read_u16::<LittleEndian>()?,
        create_date:      rdr.read_u16::<LittleEndian>()?,
        access_date:      rdr.read_u16::<LittleEndian>()?,
        first_cluster_hi: rdr.read_u16::<LittleEndian>()?,
        modify_time:      rdr.read_u16::<LittleEndian>()?,
        modify_date:      rdr.read_u16::<LittleEndian>()?,
        first_cluster_lo: rdr.read_u16::<LittleEndian>()?,
        size:             rdr.read_u32::<LittleEndian>()?,
    })
}
