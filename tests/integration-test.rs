extern crate rustfat;

use std::fs::File;
use std::io::BufReader;
use std::io::prelude::*;
use std::str;

use rustfat::FatFileSystem;

const TEST_TEXT: &'static str = "Rust is cool!\n";

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
    
    {
        let mut short_file = entries[1].get_file();
        let mut buf = Vec::new();
        short_file.read_to_end(&mut buf).unwrap();
        assert_eq!(str::from_utf8(&buf).unwrap(), TEST_TEXT);
    }
    
    {
        let mut long_file = entries[0].get_file();
        let mut buf = Vec::new();
        long_file.read_to_end(&mut buf).unwrap();
        assert_eq!(str::from_utf8(&buf).unwrap(), TEST_TEXT.repeat(1000));
    }
    
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
