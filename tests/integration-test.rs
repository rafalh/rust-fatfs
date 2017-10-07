extern crate fatfs;

use std::fs;
use std::io::{BufReader, SeekFrom};
use std::io::prelude::*;
use std::str;

use fatfs::{FileSystem, FatType, DirEntry};

const TEST_TEXT: &str = "Rust is cool!\n";
const FAT12_IMG: &str = "resources/fat12.img";
const FAT16_IMG: &str = "resources/fat16.img";
const FAT32_IMG: &str = "resources/fat32.img";

fn call_with_fs(f: &Fn(FileSystem) -> (), filename: &str) {
    let file = fs::File::open(filename).unwrap();
    let mut buf_rdr = BufReader::new(file);
    let fs = FileSystem::new(&mut buf_rdr).unwrap();
    f(fs);
}

fn test_root_dir(fs: FileSystem) {
    let root_dir = fs.root_dir();
    let entries = root_dir.iter().map(|r| r.unwrap()).collect::<Vec<DirEntry>>();
    let short_names = entries.iter().map(|e| e.short_file_name()).collect::<Vec<String>>();
    assert_eq!(short_names, ["LONG.TXT", "SHORT.TXT", "VERY", "VERY-L~1"]);
    let names = entries.iter().map(|e| e.file_name()).collect::<Vec<String>>();
    assert_eq!(names, ["long.txt", "short.txt", "very", "very-long-dir-name"]);
    // Try read again
    let names2 = root_dir.iter().map(|r| r.unwrap().file_name()).collect::<Vec<String>>();
    assert_eq!(names2, names);
}

#[test]
fn test_root_dir_fat12() {
    call_with_fs(&test_root_dir, FAT12_IMG)
}

#[test]
fn test_root_dir_fat16() {
    call_with_fs(&test_root_dir, FAT16_IMG)
}

#[test]
fn test_root_dir_fat32() {
    call_with_fs(&test_root_dir, FAT32_IMG)
}

fn test_read_seek_short_file(fs: FileSystem) {
    let mut root_dir = fs.root_dir();
    let mut short_file = root_dir.open_file("short.txt").unwrap();
    let mut buf = Vec::new();
    short_file.read_to_end(&mut buf).unwrap();
    assert_eq!(str::from_utf8(&buf).unwrap(), TEST_TEXT);
    
    assert_eq!(short_file.seek(SeekFrom::Start(5)).unwrap(), 5);
    let mut buf2 = [0; 5];
    short_file.read_exact(&mut buf2).unwrap();
    assert_eq!(str::from_utf8(&buf2).unwrap(), &TEST_TEXT[5..10]);
    
    assert_eq!(short_file.seek(SeekFrom::Start(1000)).unwrap(), 1000);
    let mut buf2 = [0; 5];
    assert_eq!(short_file.read(&mut buf2).unwrap(), 0);
}

#[test]
fn test_read_seek_short_file_fat12() {
    call_with_fs(&test_read_seek_short_file, FAT12_IMG)
}

#[test]
fn test_read_seek_short_file_fat16() {
    call_with_fs(&test_read_seek_short_file, FAT16_IMG)
}

#[test]
fn test_read_seek_short_file_fat32() {
    call_with_fs(&test_read_seek_short_file, FAT32_IMG)
}

fn test_read_long_file(fs: FileSystem) {
    let mut root_dir = fs.root_dir();
    let mut long_file = root_dir.open_file("long.txt").unwrap();
    let mut buf = Vec::new();
    long_file.read_to_end(&mut buf).unwrap();
    assert_eq!(str::from_utf8(&buf).unwrap(), TEST_TEXT.repeat(1000));
    
    assert_eq!(long_file.seek(SeekFrom::Start(2017)).unwrap(), 2017);
    buf.clear();
    let mut buf2 = [0; 10];
    long_file.read_exact(&mut buf2).unwrap();
    assert_eq!(str::from_utf8(&buf2).unwrap(), &TEST_TEXT.repeat(1000)[2017..2027]);
}

#[test]
fn test_read_long_file_fat12() {
    call_with_fs(&test_read_long_file, FAT12_IMG)
}

#[test]
fn test_read_long_file_fat16() {
    call_with_fs(&test_read_long_file, FAT16_IMG)
}

#[test]
fn test_read_long_file_fat32() {
    call_with_fs(&test_read_long_file, FAT32_IMG)
}

fn test_get_dir_by_path(fs: FileSystem) {
    let mut root_dir = fs.root_dir();
    let dir = root_dir.open_dir("very/long/path/").unwrap();
    let names = dir.iter().map(|r| r.unwrap().file_name()).collect::<Vec<String>>();
    assert_eq!(names, [".", "..", "test.txt"]);
    
    let dir2 = root_dir.open_dir("very/long/path/././.").unwrap();
    let names2 = dir2.iter().map(|r| r.unwrap().file_name()).collect::<Vec<String>>();
    assert_eq!(names2, [".", "..", "test.txt"]);
    
    let root_dir2 = root_dir.open_dir("very/long/path/../../..").unwrap();
    let root_names = root_dir2.iter().map(|r| r.unwrap().file_name()).collect::<Vec<String>>();
    let root_names2 = root_dir.iter().map(|r| r.unwrap().file_name()).collect::<Vec<String>>();
    assert_eq!(root_names, root_names2);
}

#[test]
fn test_get_dir_by_path_fat12() {
    call_with_fs(&test_get_dir_by_path, FAT12_IMG)
}

#[test]
fn test_get_dir_by_path_fat16() {
    call_with_fs(&test_get_dir_by_path, FAT16_IMG)
}

#[test]
fn test_get_dir_by_path_fat32() {
    call_with_fs(&test_get_dir_by_path, FAT32_IMG)
}

fn test_get_file_by_path(fs: FileSystem) {
    let mut root_dir = fs.root_dir();
    let mut file = root_dir.open_file("very/long/path/test.txt").unwrap();
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).unwrap();
    assert_eq!(str::from_utf8(&buf).unwrap(), TEST_TEXT);
    
    let mut file = root_dir.open_file("very-long-dir-name/very-long-file-name.txt").unwrap();
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).unwrap();
    assert_eq!(str::from_utf8(&buf).unwrap(), TEST_TEXT);
}

#[test]
fn test_get_file_by_path_fat12() {
    call_with_fs(&test_get_file_by_path, FAT12_IMG)
}

#[test]
fn test_get_file_by_path_fat16() {
    call_with_fs(&test_get_file_by_path, FAT16_IMG)
}

#[test]
fn test_get_file_by_path_fat32() {
    call_with_fs(&test_get_file_by_path, FAT32_IMG)
}

fn test_volume_metadata(fs: FileSystem, fat_type: FatType) {
    assert_eq!(fs.volume_id(), 0x12345678);
    assert_eq!(fs.volume_label(), "Test!");
    assert_eq!(fs.fat_type(), fat_type);
}

#[test]
fn test_volume_metadata_fat12() {
    call_with_fs(&|fs| test_volume_metadata(fs, FatType::Fat12), FAT12_IMG)
}

#[test]
fn test_volume_metadata_fat16() {
    call_with_fs(&|fs| test_volume_metadata(fs, FatType::Fat16), FAT16_IMG)
}

#[test]
fn test_volume_metadata_fat32() {
    call_with_fs(&|fs| test_volume_metadata(fs, FatType::Fat32), FAT32_IMG)
}
