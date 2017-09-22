extern crate byteorder;
extern crate chrono;

use std::fs::File;
use std::io::BufReader;
use std::io;
use std::str;
use fs::FatFileSystem;

pub mod fs;
pub mod dir;
pub mod file;

fn fat_test() -> io::Result<()> {
    let file = File::open("resources/floppy.img")?;
    let mut buf_rdr = BufReader::new(file);
    let mut fs = FatFileSystem::new(&mut buf_rdr)?;
    let mut root_dir = fs.root_dir();
    let entries = fs.read_dir(&mut root_dir)?;
    for e in entries {
        println!("{} - size {} - modified {}", e.get_name(), e.get_size(), e.get_modify_time());
        //println!("name {} size {} cluster {}", name_str, entry.size, entry.first_cluster_lo);
    }
    Ok(())
}

fn main() {
    println!("FAT test!");
    fat_test().unwrap();
}
