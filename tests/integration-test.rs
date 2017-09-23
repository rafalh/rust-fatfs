extern crate rustfat;

use std::fs::File;
use std::io::BufReader;
use std::str;

use rustfat::FatFileSystem;

#[test]
fn fat12_test() {
    let file = File::open("resources/floppy.img").unwrap();
    let buf_rdr = BufReader::new(file);
    let mut fs = FatFileSystem::new(Box::new(buf_rdr)).unwrap();
    let mut root_dir = fs.root_dir();
    let entries = root_dir.list().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].get_name(), "RAFOS");
    assert_eq!(entries[1].get_name(), "GRUB");
}
