use std::fs::File;
use std::io::prelude::*;
use std::io::BufReader;
use std::io;
use std::str;
use std::io::{Error, ErrorKind, SeekFrom};
use byteorder::{LittleEndian, ReadBytesExt};
use fs::{FatFileSystem, FatType};

#[derive(Debug, PartialEq)]
#[allow(dead_code)]
enum FatFileAttribute {
    READ_ONLY = 0x01,
    HIDDEN    = 0x02,
    SYSTEM    = 0x04,
    VOLUME_ID = 0x08,
    DIRECTORY = 0x10,
    ARCHIVE   = 0x20,
    LFN       = 0x0F,
}

#[allow(dead_code)]
pub struct FatDirEntry {
    name: [u8; 11],
    attrs: u8,
    reserved_0: u8,
    creation_time_0: u8,
    creation_time_1: u16,
    creation_date: u16,
    access_date: u16,
    first_cluster_hi: u16,
    mod_time: u16,
    mod_date: u16,
    first_cluster_lo: u16,
    size: u32,
}

pub struct FatDir {
    cluster: u32,
}

impl FatDir {
    pub fn new(cluster: u32) -> FatDir {
        FatDir {
            cluster: cluster,
        }
    }
}

trait DirEntry {
    fn get_name(&self) -> str;
}

trait ReadDir {
    fn read_dir(&mut self, dir: &FatDir) -> io::Result<Vec<FatDirEntry>>;
}

impl<T: Read+Seek> FatFileSystem<T> {
    pub fn read_dir(&mut self, dir: FatDir) -> io::Result<Vec<FatDirEntry>> {
        let mut entries = Vec::new();
        loop {
            let entry = read_dir_entry(&mut self.rdr)?;
            if entry.name[0] == 0 {
                break; // end of dir
            }
            if entry.name[0] == 0xE5 {
                continue; // deleted
            }
            let name_str = str::from_utf8(&entry.name).unwrap().trim_right();
            println!("name {} size {} cluster {}", name_str, entry.size, entry.first_cluster_lo);
        }
        Ok(entries)
    }
}

fn read_dir_entry(rdr: &mut Read) -> io::Result<FatDirEntry> {
    let mut name = [0; 11];
    rdr.read(&mut name)?;
    Ok(FatDirEntry {
        name:             name,
        attrs:            rdr.read_u8()?,
        reserved_0:       rdr.read_u8()?,
        creation_time_0:  rdr.read_u8()?,
        creation_time_1:  rdr.read_u16::<LittleEndian>()?,
        creation_date:    rdr.read_u16::<LittleEndian>()?,
        access_date:      rdr.read_u16::<LittleEndian>()?,
        first_cluster_hi: rdr.read_u16::<LittleEndian>()?,
        mod_time:         rdr.read_u16::<LittleEndian>()?,
        mod_date:         rdr.read_u16::<LittleEndian>()?,
        first_cluster_lo: rdr.read_u16::<LittleEndian>()?,
        size:             rdr.read_u32::<LittleEndian>()?,
    })
}

// impl FatDir {
//     pub fn new(rdr: &mut Read) -> io::Result<FatDir> {
//         let dir = FatDir {
//             entries: Vec::new(),
//         };
//         read_dir_entry(rdr)?;
//         Ok(dir)
//     }
// 
//     pub fn print(&mut self) -> io::Result<()> {
//         //let pos = self.rdr.seek(SeekFrom::Current(0))?;
//         //println!("Reading dir at {}", pos);
//         loop {
//             let entry = self.read_dir_entry()?;
//             if entry.name[0] == 0 {
//                 break; // end of dir
//             }
//             if entry.name[0] == 0xE5 {
//                 continue; // deleted
//             }
//             let name_str = str::from_utf8(&entry.name).unwrap().trim_right();
//             println!("name {} size {} cluster {}", name_str, entry.size, entry.first_cluster_lo);
//         }
//         Ok(())
//     }
// }
