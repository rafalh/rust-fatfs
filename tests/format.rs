extern crate env_logger;
extern crate fatfs;
extern crate fscommon;

use std::io;
use std::io::prelude::*;

use fscommon::BufStream;

const KB: u32 = 1024;
const MB: u32 = KB * 1024;
const TEST_STR: &str = "Hi there Rust programmer!\n";

type FileSystem = fatfs::FileSystem<BufStream<io::Cursor<Vec<u8>>>>;

fn basic_fs_test(fs: &FileSystem) {
    let root_dir = fs.root_dir();
    let entries = root_dir.iter().map(|r| r.unwrap()).collect::<Vec<_>>();
    assert_eq!(entries.len(), 0);

    let mut file = root_dir.create_file("short.txt").expect("create file");
    file.truncate().expect("truncate file");
    file.write_all(&TEST_STR.as_bytes()).expect("write file");

    let entries = root_dir.iter().map(|r| r.unwrap()).collect::<Vec<_>>();
    assert_eq!(entries.len(), 1);
}

fn test_format_fs(opts: fatfs::FormatOptions, total_bytes: u64) -> FileSystem {
    let _ = env_logger::try_init();
    let storage_vec: Vec<u8> = Vec::with_capacity(total_bytes as usize);
    let storage_cur = io::Cursor::new(storage_vec);
    let mut buffered_stream = BufStream::new(storage_cur);
    fatfs::format_volume(&mut buffered_stream, opts).expect("format volume");

    let fs = fatfs::FileSystem::new(buffered_stream, fatfs::FsOptions::new()).expect("open fs");
    basic_fs_test(&fs);
    fs
}

#[test]
fn test_format_1mb() {
    let total_bytes = 1 * MB as u64;
    let mut opts: fatfs::FormatOptions = Default::default();
    opts.total_sectors = (total_bytes / 512) as u32;
    let fs = test_format_fs(opts, total_bytes);
    assert_eq!(fs.fat_type(), fatfs::FatType::Fat12);
}

#[test]
fn test_format_8mb() {
    let total_bytes = 8 * MB as u64;
    let mut opts: fatfs::FormatOptions = Default::default();
    opts.total_sectors = (total_bytes / 512) as u32;
    let fs = test_format_fs(opts, total_bytes);
    assert_eq!(fs.fat_type(), fatfs::FatType::Fat16);
}

#[test]
fn test_format_50mb() {
    let total_bytes = 50 * MB as u64;
    let mut opts: fatfs::FormatOptions = Default::default();
    opts.total_sectors = (total_bytes / 512) as u32;
    let fs = test_format_fs(opts, total_bytes);
    assert_eq!(fs.fat_type(), fatfs::FatType::Fat16);
}


#[test]
fn test_format_512mb() {
    let total_bytes = 2 * 1024 * MB as u64;
    let mut opts: fatfs::FormatOptions = Default::default();
    opts.total_sectors = (total_bytes / 512) as u32;
    let fs = test_format_fs(opts, total_bytes);
    assert_eq!(fs.fat_type(), fatfs::FatType::Fat32);
}
