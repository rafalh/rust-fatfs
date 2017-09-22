extern crate byteorder;

use std::fs::File;
use std::io::prelude::*;
use std::io::BufReader;
use std::io;
use std::str;
use std::io::{Error, ErrorKind, SeekFrom};
use byteorder::{LittleEndian, ReadBytesExt};
use fs::FatFileSystem;

mod fs;
mod dir;

fn fat_test() -> io::Result<()> {
    let file = File::open("resources/floppy.img")?;
    let mut buf_rdr = BufReader::new(file);
    let mut fs = FatFileSystem::new(&mut buf_rdr)?;
    let root_dir = fs.open_root_dir()?;
    fs.read_dir(root_dir)?;
    Ok(())
}

fn main() {
    println!("FAT test!");
    fat_test().unwrap();
}
