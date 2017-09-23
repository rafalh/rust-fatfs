extern crate rustfat;

use std::fs::File;
use std::io::BufReader;
use std::str;

use rustfat::FatFileSystem;

fn test_img(name: &str) {
    let file = File::open(name).unwrap();
    let buf_rdr = BufReader::new(file);
    let mut fs = FatFileSystem::new(Box::new(buf_rdr)).unwrap();
    let mut root_dir = fs.root_dir();
    let entries = root_dir.list().unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].get_name(), "LONG.TXT");
    assert_eq!(entries[1].get_name(), "SHORT.TXT");
    assert_eq!(entries[2].get_name(), "VERY");
}

#[test]
fn fat12() {
    test_img("resources/fat12.img");
}

#[test]
fn fat16() {
    test_img("resources/fat16.img");
}

#[test]
fn fat32() {
    test_img("resources/fat32.img");
}
