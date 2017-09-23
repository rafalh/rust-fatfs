extern crate rustfat;

use std::fs::File;
use std::io::BufReader;
use std::str;

use rustfat::FatFileSystem;

fn main() {
    let file = File::open("resources/fat32.img").unwrap();
    let buf_rdr = BufReader::new(file);
    let mut fs = FatFileSystem::new(Box::new(buf_rdr)).unwrap();
    let mut root_dir = fs.root_dir();
    let entries = root_dir.list().unwrap();
    for e in entries {
        println!("{} - size {} - modified {}", e.get_name(), e.get_size(), e.get_modify_time());
    }
}
