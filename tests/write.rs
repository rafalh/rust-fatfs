extern crate fatfs;
extern crate env_logger;

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
    let _ = env_logger::init();
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
    file.write_all(&TEST_STR.as_bytes()).unwrap();
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
    file.write_all(&test_str.as_bytes()).unwrap();
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

fn test_remove(fs: FileSystem) {
    let mut root_dir = fs.root_dir();
    assert!(root_dir.remove("very/long/path").is_err());
    let dir = root_dir.open_dir("very/long/path").unwrap();
    let mut names = dir.iter().map(|r| r.unwrap().file_name()).collect::<Vec<String>>();
    assert_eq!(names, [".", "..", "test.txt"]);
    root_dir.remove("very/long/path/test.txt").unwrap();
    names = dir.iter().map(|r| r.unwrap().file_name()).collect::<Vec<String>>();
    assert_eq!(names, [".", ".."]);
    assert!(root_dir.remove("very/long/path").is_ok());
    
    names = root_dir.iter().map(|r| r.unwrap().file_name()).collect::<Vec<String>>();
    assert_eq!(names, ["long.txt", "short.txt", "very", "very-long-dir-name"]);
    root_dir.remove("long.txt").unwrap();
    names = root_dir.iter().map(|r| r.unwrap().file_name()).collect::<Vec<String>>();
    assert_eq!(names, ["short.txt", "very", "very-long-dir-name"]);
}

#[test]
fn test_remove_fat12() {
    call_with_fs(&test_remove, FAT12_IMG, 3)
}

#[test]
fn test_remove_fat16() {
    call_with_fs(&test_remove, FAT16_IMG, 3)
}

#[test]
fn test_remove_fat32() {
    call_with_fs(&test_remove, FAT32_IMG, 3)
}

fn test_create_file(fs: FileSystem) {
    let mut root_dir = fs.root_dir();
    let dir = root_dir.open_dir("very/long/path").unwrap();
    let mut names = dir.iter().map(|r| r.unwrap().file_name()).collect::<Vec<String>>();
    assert_eq!(names, [".", "..", "test.txt"]);
    {
        let mut file = root_dir.create_file("very/long/path/new-file-with-long-name.txt").unwrap();
        file.write_all(&TEST_STR.as_bytes()).unwrap();
    }
    names = dir.iter().map(|r| r.unwrap().file_name()).collect::<Vec<String>>();
    assert_eq!(names, [".", "..", "test.txt", "new-file-with-long-name.txt"]);
    {
        let mut file = root_dir.open_file("very/long/path/new-file-with-long-name.txt").unwrap();
        let mut content = String::new();
        file.read_to_string(&mut content).unwrap();
        assert_eq!(&content, &TEST_STR);
    }
}

#[test]
fn test_create_file_fat12() {
    call_with_fs(&test_create_file, FAT12_IMG, 4)
}

#[test]
fn test_create_file_fat16() {
    call_with_fs(&test_create_file, FAT16_IMG, 4)
}

#[test]
fn test_create_file_fat32() {
    call_with_fs(&test_create_file, FAT32_IMG, 4)
}
