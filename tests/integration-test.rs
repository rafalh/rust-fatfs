extern crate rfat;

use std::fs::File;
use std::io::{BufReader, SeekFrom};
use std::io::prelude::*;
use std::str;

use rfat::FatFileSystem;

const TEST_TEXT: &'static str = "Rust is cool!\n";
const FAT12_IMG: &'static str = "resources/fat12.img";
const FAT16_IMG: &'static str = "resources/fat16.img";
const FAT32_IMG: &'static str = "resources/fat32.img";

fn open_fs(filename: &str) -> FatFileSystem {
    let file = File::open(filename).unwrap();
    let buf_rdr = BufReader::new(file);
    FatFileSystem::new(Box::new(buf_rdr)).unwrap()
}

fn test_root_dir(mut fs: FatFileSystem) {
    let mut root_dir = fs.root_dir();
    let entries = root_dir.list().unwrap();
    let names = entries.iter().map(|e| e.file_name()).collect::<Vec<String>>();
    assert_eq!(names, ["LONG.TXT", "SHORT.TXT", "VERY"]);
    // Try read again
    let entries = root_dir.list().unwrap();
    let names2 = entries.iter().map(|e| e.file_name()).collect::<Vec<String>>();
    assert_eq!(names2, names);
}

#[test]
fn test_root_dir_fat12() {
    test_root_dir(open_fs(FAT12_IMG));
}

#[test]
fn test_root_dir_fat16() {
    test_root_dir(open_fs(FAT16_IMG));
}

#[test]
fn test_root_dir_fat32() {
    test_root_dir(open_fs(FAT32_IMG));
}

fn test_read_seek_short_file(mut fs: FatFileSystem) {
    let mut root_dir = fs.root_dir();
    let mut short_file = root_dir.get_file("short.txt").unwrap();
    let mut buf = Vec::new();
    short_file.read_to_end(&mut buf).unwrap();
    assert_eq!(str::from_utf8(&buf).unwrap(), TEST_TEXT);
    
    short_file.seek(SeekFrom::Start(5)).unwrap();
    let mut buf2 = [0; 5];
    short_file.read_exact(&mut buf2).unwrap();
    assert_eq!(str::from_utf8(&buf2).unwrap(), &TEST_TEXT[5..10]);
}

#[test]
fn test_read_seek_short_file_fat12() {
    test_read_seek_short_file(open_fs(FAT12_IMG))
}

#[test]
fn test_read_seek_short_file_fat16() {
    test_read_seek_short_file(open_fs(FAT16_IMG))
}

#[test]
fn test_read_seek_short_file_fat32() {
    test_read_seek_short_file(open_fs(FAT32_IMG))
}

fn test_read_long_file(mut fs: FatFileSystem) {
    let mut root_dir = fs.root_dir();
    let mut long_file = root_dir.get_file("long.txt").unwrap();
    let mut buf = Vec::new();
    long_file.read_to_end(&mut buf).unwrap();
    assert_eq!(str::from_utf8(&buf).unwrap(), TEST_TEXT.repeat(1000));
    
    long_file.seek(SeekFrom::Start(2017)).unwrap();
    buf.clear();
    let mut buf2 = [0; 10];
    long_file.read_exact(&mut buf2).unwrap();
    assert_eq!(str::from_utf8(&buf2).unwrap(), &TEST_TEXT.repeat(1000)[2017..2027]);
}

#[test]
fn test_read_long_file_fat12() {
    test_read_long_file(open_fs(FAT12_IMG))
}

#[test]
fn test_read_long_file_fat16() {
    test_read_long_file(open_fs(FAT16_IMG))
}

#[test]
fn test_read_long_file_fat32() {
    test_read_long_file(open_fs(FAT32_IMG))
}

fn test_get_dir_by_path(mut fs: FatFileSystem) {
    let mut root_dir = fs.root_dir();
    let mut dir = root_dir.get_dir("very/long/path/").unwrap();
    let entries = dir.list().unwrap();
    let names = entries.iter().map(|e| e.file_name()).collect::<Vec<String>>();
    assert_eq!(names, [".", "..", "TEST.TXT"]);
}

#[test]
fn test_get_dir_by_path_fat12() {
    test_get_dir_by_path(open_fs(FAT12_IMG))
}

#[test]
fn test_get_dir_by_path_fat16() {
    test_get_dir_by_path(open_fs(FAT16_IMG))
}

#[test]
fn test_get_dir_by_path_fat32() {
    test_get_dir_by_path(open_fs(FAT32_IMG))
}

fn test_get_file_by_path(mut fs: FatFileSystem) {
    let mut root_dir = fs.root_dir();
    let mut file = root_dir.get_file("very/long/path/test.txt").unwrap();
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).unwrap();
    assert_eq!(str::from_utf8(&buf).unwrap(), TEST_TEXT);
}

#[test]
fn test_get_file_by_path_fat12() {
    test_get_file_by_path(open_fs(FAT12_IMG))
}

#[test]
fn test_get_file_by_path_fat16() {
    test_get_file_by_path(open_fs(FAT16_IMG))
}

#[test]
fn test_get_file_by_path_fat32() {
    test_get_file_by_path(open_fs(FAT32_IMG))
}
