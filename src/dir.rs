use std::ascii::AsciiExt;
use std::io::prelude::*;
use std::io;
use std::io::{ErrorKind, SeekFrom};
use std::str;
use byteorder::{LittleEndian, ReadBytesExt};
use chrono::{DateTime, Date, TimeZone, Local};

use fs::{FatSharedStateRef, ReadSeek};
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
#[derive(Clone)]
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
    state: FatSharedStateRef,
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
        let name = str::from_utf8(&self.name[0..8]).unwrap().trim_right();
        let ext = str::from_utf8(&self.name[8..11]).unwrap().trim_right();
        if ext == "" { name.to_string() } else { format!("{}.{}", name, ext) }
    }
    
    pub fn get_attrs(&self) -> FatFileAttributes {
        self.attrs
    }
    
    pub fn is_dir(&self) -> bool {
        self.attrs.contains(FatFileAttributes::DIRECTORY)
    }
    
    pub fn get_cluster(&self) -> u32 {
        ((self.first_cluster_hi as u32) << 16) | self.first_cluster_lo as u32
    }
    
    pub fn get_file(&self) -> FatFile {
        if self.is_dir() {
            panic!("This is a directory");
        }
        FatFile::new(self.get_cluster(), Some(self.size), self.state.clone())
    }
    
    pub fn get_dir(&self) -> FatDir {
        if !self.is_dir() {
            panic!("This is a file");
        }
        let file = FatFile::new(self.get_cluster(), None, self.state.clone());
        FatDir::new(Box::new(file), self.state.clone())
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

pub struct FatDir {
    rdr: Box<ReadSeek>,
    state: FatSharedStateRef,
}

impl FatDir {
    
    pub(crate) fn new(rdr: Box<ReadSeek>, state: FatSharedStateRef) -> FatDir {
        FatDir { rdr, state }
    }
    
    pub fn list(&mut self) -> io::Result<Vec<FatDirEntry>> {
        self.rewind();
        Ok(self.map(|x| x.unwrap()).collect())
    }
    
    pub fn rewind(&mut self) {
        self.rdr.seek(SeekFrom::Start(0)).unwrap();
    }
    
    fn read_dir_entry(&mut self) -> io::Result<FatDirEntry> {
        let mut name = [0; 11];
        self.rdr.read(&mut name)?;
        let attrs = FatFileAttributes::from_bits(self.rdr.read_u8()?).expect("invalid attributes");
        Ok(FatDirEntry {
            name,
            attrs,
            reserved_0:       self.rdr.read_u8()?,
            create_time_0:    self.rdr.read_u8()?,
            create_time_1:    self.rdr.read_u16::<LittleEndian>()?,
            create_date:      self.rdr.read_u16::<LittleEndian>()?,
            access_date:      self.rdr.read_u16::<LittleEndian>()?,
            first_cluster_hi: self.rdr.read_u16::<LittleEndian>()?,
            modify_time:      self.rdr.read_u16::<LittleEndian>()?,
            modify_date:      self.rdr.read_u16::<LittleEndian>()?,
            first_cluster_lo: self.rdr.read_u16::<LittleEndian>()?,
            size:             self.rdr.read_u32::<LittleEndian>()?,
            state:            self.state.clone(),
        })
    }
    
    fn split_path<'a>(path: &'a str) -> (&'a str, Option<&'a str>) {
        let mut path_split = path.trim_matches('/').splitn(2, "/");
        let comp = path_split.next().unwrap();
        let rest_opt = path_split.next();
        (comp, rest_opt)
    }
    
    fn find_entry(&mut self, name: &str) -> io::Result<FatDirEntry> {
        let entries: Vec<FatDirEntry> = self.list()?;
        for e in entries {
            if e.get_name().eq_ignore_ascii_case(name) {
                println!("find entry {}", name);
                return Ok(e);
            }
        }
        Err(io::Error::new(ErrorKind::NotFound, "file not found"))
    }
    
    pub fn get_dir(&mut self, path: &str) -> io::Result<FatDir> {
        let (name, rest_opt) = Self::split_path(path);
        let e = self.find_entry(name)?;
        match rest_opt {
            Some(rest) => e.get_dir().get_dir(rest),
            None => Ok(e.get_dir())
        }
    }
    
    pub fn get_file(&mut self, path: &str) -> io::Result<FatFile> {
        let (name, rest_opt) = Self::split_path(path);
        let e = self.find_entry(name)?;
        match rest_opt {
            Some(rest) => e.get_dir().get_file(rest),
            None => Ok(e.get_file())
        }
    }
}

impl Iterator for FatDir {
    type Item = io::Result<FatDirEntry>;

    fn next(&mut self) -> Option<io::Result<FatDirEntry>> {
        loop {
            let r = self.read_dir_entry();
            let e = match r {
                Ok(e) => e,
                Err(_) => return Some(r),
            };
            if e.name[0] == 0 {
                return None; // end of dir
            }
            if e.name[0] == 0xE5 {
                continue; // deleted
            }
            if e.attrs == FatFileAttributes::LFN {
                continue; // FIXME: support LFN
            }
            return Some(Ok(e))
        }
    }
}
