extern crate env_logger;
extern crate fatfs;
extern crate fscommon;

use std::io;
use std::io::prelude::*;

use fscommon::BufStream;

const KB: u64 = 1024;
const MB: u64 = KB * 1024;
const TEST_STR: &str = "Hi there Rust programmer!\n";

type FileSystem = fatfs::FileSystem<BufStream<io::Cursor<Vec<u8>>>>;

fn basic_fs_test(fs: &FileSystem) {
    let root_dir = fs.root_dir();
    let entries = root_dir.iter().map(|r| r.unwrap()).collect::<Vec<_>>();
    assert_eq!(entries.len(), 0);

    let subdir1 = root_dir.create_dir("subdir1").expect("create_dir subdir1");
    let subdir2 = root_dir.create_dir("subdir1/subdir2 with long name").expect("create_dir subdir2");

    let test_str = TEST_STR.repeat(1000);
    {
        let mut file = subdir2.create_file("test file name.txt").expect("create file");
        file.truncate().expect("truncate file");
        file.write_all(test_str.as_bytes()).expect("write file");
    }

    let mut file = root_dir.open_file("subdir1/subdir2 with long name/test file name.txt").unwrap();
    let mut content = String::new();
    file.read_to_string(&mut content).expect("read_to_string");
    assert_eq!(content, test_str);

    let filenames = root_dir.iter().map(|r| r.unwrap().file_name()).collect::<Vec<String>>();
    assert_eq!(filenames, ["subdir1"]);

    let filenames = subdir2.iter().map(|r| r.unwrap().file_name()).collect::<Vec<String>>();
    assert_eq!(filenames, [".", "..", "test file name.txt"]);

    subdir1.rename("subdir2 with long name/test file name.txt", &root_dir, "new-name.txt").expect("rename");

    let filenames = subdir2.iter().map(|r| r.unwrap().file_name()).collect::<Vec<String>>();
    assert_eq!(filenames, [".", ".."]);

    let filenames = root_dir.iter().map(|r| r.unwrap().file_name()).collect::<Vec<String>>();
    assert_eq!(filenames, ["subdir1", "new-name.txt"]);
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
    let total_bytes = 1 * MB;
    let mut opts: fatfs::FormatOptions = Default::default();
    opts.total_sectors = (total_bytes / 512) as u32;
    let fs = test_format_fs(opts, total_bytes);
    assert_eq!(fs.fat_type(), fatfs::FatType::Fat12);
}

#[test]
fn test_format_8mb() {
    let total_bytes = 8 * MB;
    let mut opts: fatfs::FormatOptions = Default::default();
    opts.total_sectors = (total_bytes / 512) as u32;
    let fs = test_format_fs(opts, total_bytes);
    assert_eq!(fs.fat_type(), fatfs::FatType::Fat16);
}

#[test]
fn test_format_50mb() {
    let total_bytes = 50 * MB;
    let mut opts: fatfs::FormatOptions = Default::default();
    opts.total_sectors = (total_bytes / 512) as u32;
    let fs = test_format_fs(opts, total_bytes);
    assert_eq!(fs.fat_type(), fatfs::FatType::Fat16);
}


#[test]
fn test_format_512mb() {
    let total_bytes = 2 * 1024 * MB;
    let mut opts: fatfs::FormatOptions = Default::default();
    opts.total_sectors = (total_bytes / 512) as u32;
    let fs = test_format_fs(opts, total_bytes);
    assert_eq!(fs.fat_type(), fatfs::FatType::Fat32);
}

fn create_format_options(total_bytes: u64, bytes_per_sector: Option<u16>) -> fatfs::FormatOptions {
    let mut opts: fatfs::FormatOptions = Default::default();
    opts.total_sectors = (total_bytes / bytes_per_sector.unwrap_or(512) as u64) as u32;
    opts.bytes_per_sector = bytes_per_sector;
    opts
}

#[test]
fn test_format_512mb_4096sec() {
    let total_bytes = 1 * 1024 * MB;
    let opts = create_format_options(total_bytes, Some(1024));
    let fs = test_format_fs(opts, total_bytes);
    assert_eq!(fs.fat_type(), fatfs::FatType::Fat32);
}
