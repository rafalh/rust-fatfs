extern crate fatfs;

use std::fs;
use std::io::prelude::*;
use std::io;
use std::str;

use fatfs::FileSystem;
use fatfs::BufStream;

const FAT12_IMG: &str = "fat12.img";
const FAT16_IMG: &str = "fat16.img";
const FAT32_IMG: &str = "fat32.img";
const IMG_DIR: &str = "resources";
const TMP_DIR: &str = "tmp";
const TEST_STR: &str = "Hi there Rust programmer!\n";

fn call_with_fs(f: &Fn(FileSystem) -> (), filename: &str, test_seq: u32) {
    let img_path = format!("{}/{}", IMG_DIR, filename);
    let tmp_path = format!("{}/{}-{}", TMP_DIR, test_seq, filename);
    fs::create_dir(TMP_DIR).ok();
    fs::copy(&img_path, &tmp_path).unwrap();
    {
        let file = fs::OpenOptions::new().read(true).write(true).open(&tmp_path).unwrap();
        let mut buf_file = BufStream::new(file);
        let fs = FileSystem::new(&mut buf_file).unwrap();
        f(fs);
    }
    fs::remove_file(tmp_path).unwrap();
}

fn test_write_short_file(fs: FileSystem) {
    let mut root_dir = fs.root_dir();
    let mut file = root_dir.open_file("short.txt").expect("open file");
    file.truncate().unwrap();
    assert_eq!(TEST_STR.len(), file.write(&TEST_STR.as_bytes()).unwrap());
    file.seek(io::SeekFrom::Start(0)).unwrap();
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).unwrap();
    assert_eq!(TEST_STR, str::from_utf8(&buf).unwrap());
}

#[test]
fn test_write_file_fat12() {
    call_with_fs(&test_write_short_file, FAT12_IMG, 1)
}

#[test]
fn test_write_file_fat16() {
    call_with_fs(&test_write_short_file, FAT16_IMG, 1)
}

#[test]
fn test_write_file_fat32() {
    call_with_fs(&test_write_short_file, FAT32_IMG, 1)
}

fn test_write_long_file(fs: FileSystem) {
    let mut root_dir = fs.root_dir();
    let mut file = root_dir.open_file("long.txt").expect("open file");
    file.truncate().unwrap();
    let test_str = TEST_STR.repeat(100);
    assert_eq!(test_str.len(), file.write(&test_str.as_bytes()).unwrap());
    file.seek(io::SeekFrom::Start(0)).unwrap();
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).unwrap();
    assert_eq!(test_str, str::from_utf8(&buf).unwrap());
    file.seek(io::SeekFrom::Start(1234)).unwrap();
    file.truncate().unwrap();
    file.seek(io::SeekFrom::Start(0)).unwrap();
    buf.clear();
    file.read_to_end(&mut buf).unwrap();
    assert_eq!(&test_str[..1234], str::from_utf8(&buf).unwrap());
}

#[test]
fn test_write_long_file_fat12() {
    call_with_fs(&test_write_long_file, FAT12_IMG, 2)
}

#[test]
fn test_write_long_file_fat16() {
    call_with_fs(&test_write_long_file, FAT16_IMG, 2)
}

#[test]
fn test_write_long_file_fat32() {
    call_with_fs(&test_write_long_file, FAT32_IMG, 2)
}
