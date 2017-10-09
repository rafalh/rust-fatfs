extern crate fatfs;

use std::fs;
use std::io::prelude::*;
use std::io;
use std::str;

use fatfs::FileSystem;
// use fatfs::BufStream;

const FAT12_IMG: &str = "fat12.img";
const FAT16_IMG: &str = "fat16.img";
const FAT32_IMG: &str = "fat32.img";
const IMG_DIR: &str = "resources";
const TMP_DIR: &str = "tmp";
const TEST_STR: &str = "Hi there Rust programmer!\n";

fn call_with_fs(f: &Fn(FileSystem) -> (), filename: &str) {
    let img_path = format!("{}/{}", IMG_DIR, filename);
    let tmp_path = format!("{}/{}", TMP_DIR, filename);
    fs::create_dir(TMP_DIR).ok();
    fs::copy(&img_path, &tmp_path).unwrap();
    // let file = fs::OpenOptions::new().read(true).write(true).open(&tmp_path).unwrap();
    // let mut buf_file = BufStream::new(file);
    // let fs = FileSystem::new(&mut buf_file).unwrap();
    let mut file = fs::OpenOptions::new().read(true).write(true).open(&tmp_path).unwrap();
    let fs = FileSystem::new(&mut file).unwrap();
    f(fs);
}

fn test_write_file(fs: FileSystem) {
    let mut root_dir = fs.root_dir();
    let mut file = root_dir.open_file("short.txt").expect("open file");
    file.truncate();
    assert_eq!(TEST_STR.len(), file.write(&TEST_STR.as_bytes()).unwrap());
    file.seek(io::SeekFrom::Start(0)).unwrap();
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).unwrap();
    assert_eq!(TEST_STR, str::from_utf8(&buf).unwrap());
}

#[test]
fn test_write_file_fat12() {
    call_with_fs(&test_write_file, FAT12_IMG)
}

#[test]
fn test_write_file_fat16() {
    call_with_fs(&test_write_file, FAT16_IMG)
}

#[test]
fn test_write_file_fat32() {
    call_with_fs(&test_write_file, FAT32_IMG)
}
