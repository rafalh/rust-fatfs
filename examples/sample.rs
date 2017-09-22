extern crate rustfat;

use std::fs::File;
use std::io::BufReader;
use std::str;

use rustfat::FatFileSystem;

fn main() {
    let file = File::open("resources/floppy.img").unwrap();
    let mut buf_rdr = BufReader::new(file);
    let mut fs = FatFileSystem::new(&mut buf_rdr).unwrap();
    let mut root_dir = fs.root_dir();
    let entries = fs.read_dir(&mut root_dir).unwrap();
    for e in entries {
        println!("{} - size {} - modified {}", e.get_name(), e.get_size(), e.get_modify_time());
    }
}
