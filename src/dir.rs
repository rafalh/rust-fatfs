use std::ascii::AsciiExt;
use std::fmt;
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
#[derive(Clone, Copy, Debug)]
pub struct FatDirEntryData {
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

#[derive(Clone)]
pub struct FatDirEntry {
    data: FatDirEntryData,
    state: FatSharedStateRef,
}

impl FatDirEntry {
    pub fn file_name(&self) -> String {
        let name = str::from_utf8(&self.data.name[0..8]).unwrap().trim_right();
        let ext = str::from_utf8(&self.data.name[8..11]).unwrap().trim_right();
        if ext == "" { name.to_string() } else { format!("{}.{}", name, ext) }
    }
    
    pub fn attributes(&self) -> FatFileAttributes {
        self.data.attrs
    }
    
    pub fn is_dir(&self) -> bool {
        self.data.attrs.contains(FatFileAttributes::DIRECTORY)
    }
    
    pub fn is_file(&self) -> bool {
        !self.is_dir()
    }
    
    pub(crate) fn first_cluster(&self) -> u32 {
        ((self.data.first_cluster_hi as u32) << 16) | self.data.first_cluster_lo as u32
    }
    
    pub fn to_file(&self) -> FatFile {
        if self.is_dir() {
            panic!("This is a directory");
        }
        FatFile::new(self.first_cluster(), Some(self.data.size), self.state.clone())
    }
    
    pub fn to_dir(&self) -> FatDir {
        if !self.is_dir() {
            panic!("This is a file");
        }
        let file = FatFile::new(self.first_cluster(), None, self.state.clone());
        FatDir::new(Box::new(file), self.state.clone())
    }
    
    pub fn len(&self) -> u64 {
        self.data.size as u64
    }
    
    pub fn created(&self) -> DateTime<Local> {
        Self::convert_date_time(self.data.create_date, self.data.create_time_1)
    }
    
    pub fn accessed(&self) -> Date<Local> {
        Self::convert_date(self.data.access_date)
    }
    
    pub fn modified(&self) -> DateTime<Local> {
        Self::convert_date_time(self.data.modify_date, self.data.modify_time)
    }
    
    fn convert_date(dos_date: u16) -> Date<Local> {
        let (year, month, day) = ((dos_date >> 9) + 1980, (dos_date >> 5) & 0xF, dos_date & 0x1F);
        Local.ymd(year as i32, month as u32, day as u32)
    }
    
    fn convert_date_time(dos_date: u16, dos_time: u16) -> DateTime<Local> {
        let (hour, min, sec) = (dos_time >> 11, (dos_time >> 5) & 0x3F, (dos_time & 0x1F) * 2);
        Self::convert_date(dos_date).and_hms(hour as u32, min as u32, sec as u32)
    }
}

impl fmt::Debug for FatDirEntry {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        self.data.fmt(f)
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
    
    fn read_dir_entry_data(&mut self) -> io::Result<FatDirEntryData> {
        let mut name = [0; 11];
        self.rdr.read(&mut name)?;
        let attrs = FatFileAttributes::from_bits(self.rdr.read_u8()?).expect("invalid attributes");
        Ok(FatDirEntryData {
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
            if e.file_name().eq_ignore_ascii_case(name) {
                return Ok(e);
            }
        }
        Err(io::Error::new(ErrorKind::NotFound, "file not found"))
    }
    
    pub fn get_dir(&mut self, path: &str) -> io::Result<FatDir> {
        let (name, rest_opt) = Self::split_path(path);
        let e = self.find_entry(name)?;
        match rest_opt {
            Some(rest) => e.to_dir().get_dir(rest),
            None => Ok(e.to_dir())
        }
    }
    
    pub fn get_file(&mut self, path: &str) -> io::Result<FatFile> {
        let (name, rest_opt) = Self::split_path(path);
        let e = self.find_entry(name)?;
        match rest_opt {
            Some(rest) => e.to_dir().get_file(rest),
            None => Ok(e.to_file())
        }
    }
}

impl Iterator for FatDir {
    type Item = io::Result<FatDirEntry>;

    fn next(&mut self) -> Option<io::Result<FatDirEntry>> {
        loop {
            let res = self.read_dir_entry_data();
            let data = match res {
                Ok(data) => data,
                Err(err) => return Some(Err(err)),
            };
            if data.name[0] == 0 {
                return None; // end of dir
            }
            if data.name[0] == 0xE5 {
                continue; // deleted
            }
            if data.attrs == FatFileAttributes::LFN {
                continue; // FIXME: support LFN
            }
            return Some(Ok(FatDirEntry {
                data,
                state: self.state.clone(),
            }));
        }
    }
}
