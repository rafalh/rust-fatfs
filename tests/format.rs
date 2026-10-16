mod common;

use std::io::prelude::*;

use fatfs::FatType;

use crate::common::{create_temp_fs, init_logger, MemBackedFs as FileSystem, MB};

const TEST_STR: &str = "Hi there Rust programmer!\n";

fn format_fs(opts: fatfs::FormatVolumeOptions, total_bytes: u64) -> FileSystem {
    init_logger();
    create_temp_fs(opts, total_bytes)
}

fn basic_fs_test(fs: &FileSystem) {
    let stats = fs.stats().expect("stats");
    if fs.fat_type() == fatfs::FatType::Fat32 {
        // On FAT32 one cluster is allocated for root directory
        assert_eq!(stats.total_clusters(), stats.free_clusters() + 1);
    } else {
        assert_eq!(stats.total_clusters(), stats.free_clusters());
    }

    let root_dir = fs.root_dir();
    let entries = root_dir.iter().map(|r| r.unwrap()).collect::<Vec<_>>();
    assert_eq!(entries.len(), 0);

    let subdir1 = root_dir.create_dir("subdir1").expect("create_dir subdir1");
    let subdir2 = root_dir
        .create_dir("subdir1/subdir2 with long name")
        .expect("create_dir subdir2");

    let test_str = TEST_STR.repeat(1000);
    {
        let mut file = subdir2.create_file("test file name.txt").expect("create file");
        file.truncate().expect("truncate file");
        file.write_all(test_str.as_bytes()).expect("write file");
    }

    let mut file = root_dir
        .open_file("subdir1/subdir2 with long name/test file name.txt")
        .unwrap();
    let mut content = String::new();
    file.read_to_string(&mut content).expect("read_to_string");
    assert_eq!(content, test_str);

    let filenames = root_dir.iter().map(|r| r.unwrap().file_name()).collect::<Vec<String>>();
    assert_eq!(filenames, ["subdir1"]);

    let filenames = subdir2.iter().map(|r| r.unwrap().file_name()).collect::<Vec<String>>();
    assert_eq!(filenames, [".", "..", "test file name.txt"]);

    subdir1
        .rename("subdir2 with long name/test file name.txt", &root_dir, "new-name.txt")
        .expect("rename");

    let filenames = subdir2.iter().map(|r| r.unwrap().file_name()).collect::<Vec<String>>();
    assert_eq!(filenames, [".", ".."]);

    let filenames = root_dir.iter().map(|r| r.unwrap().file_name()).collect::<Vec<String>>();
    assert_eq!(filenames, ["subdir1", "new-name.txt"]);
}

fn test_format_fs(opts: fatfs::FormatVolumeOptions, total_bytes: u64) -> FileSystem {
    let fs = format_fs(opts, total_bytes);
    basic_fs_test(&fs);
    fs
}

#[test]
fn test_format_1mb() {
    let total_bytes = MB;
    let opts = fatfs::FormatVolumeOptions::new();
    let fs = test_format_fs(opts, total_bytes);
    assert_eq!(fs.fat_type(), fatfs::FatType::Fat12);
}

#[test]
fn test_format_8mb_1fat() {
    let total_bytes = 8 * MB;
    let opts = fatfs::FormatVolumeOptions::new().fats(1);
    let fs = test_format_fs(opts, total_bytes);
    assert_eq!(fs.fat_type(), fatfs::FatType::Fat16);
}

#[test]
fn test_format_50mb() {
    let total_bytes = 50 * MB;
    let opts = fatfs::FormatVolumeOptions::new();
    let fs = test_format_fs(opts, total_bytes);
    assert_eq!(fs.fat_type(), fatfs::FatType::Fat16);
}

#[test]
fn test_format_2gb_512sec() {
    let total_bytes = 2 * 1024 * MB;
    let opts = fatfs::FormatVolumeOptions::new();
    let fs = test_format_fs(opts, total_bytes);
    assert_eq!(fs.fat_type(), fatfs::FatType::Fat32);
}

#[test]
fn test_format_1gb_4096sec() {
    let total_bytes = 1024 * MB;
    let opts = fatfs::FormatVolumeOptions::new().bytes_per_sector(4096);
    let fs = test_format_fs(opts, total_bytes);
    assert_eq!(fs.fat_type(), fatfs::FatType::Fat32);
}

#[test]
fn test_format_empty_volume_label() {
    let total_bytes = 2 * 1024 * MB;
    let opts = fatfs::FormatVolumeOptions::new();
    let fs = test_format_fs(opts, total_bytes);
    assert_eq!(fs.volume_label(), "NO NAME");
    assert_eq!(fs.read_volume_label_from_root_dir().unwrap(), None);
}

#[test]
fn test_format_volume_label_and_id() {
    let total_bytes = 2 * 1024 * MB;
    let opts = fatfs::FormatVolumeOptions::new()
        .volume_id(1234)
        .volume_label(*b"VOLUMELABEL");
    let fs = test_format_fs(opts, total_bytes);
    assert_eq!(fs.volume_label(), "VOLUMELABEL");
    assert_eq!(
        fs.read_volume_label_from_root_dir().unwrap(),
        Some("VOLUMELABEL".to_string())
    );
    assert_eq!(fs.volume_id(), 1234);
}

#[test]
fn test_zero_root_dir_clusters() {
    init_logger();
    let total_bytes = 33 * MB;
    let opts = fatfs::FormatVolumeOptions::new().fat_type(FatType::Fat32);
    let fs = format_fs(opts, total_bytes);
    let root_dir = fs.root_dir();

    // create a bunch of files to force allocation of second root directory cluster (64 is combined size of LFN + SFN)
    let files_to_create = fs.cluster_size() as usize / 64 + 1;
    for i in 0..files_to_create {
        root_dir.create_file(&format!("f{}", i)).unwrap();
    }
    assert_eq!(root_dir.iter().count(), files_to_create);
}

#[test]
fn test_format_hidden_sectors() {
    init_logger();
    for (fat_type, total_bytes) in [(FatType::Fat16, 64 * MB), (FatType::Fat32, 64 * MB)] {
        let storage = std::io::Cursor::new(vec![0_u8; total_bytes as usize]);
        let mut stream = fatfs::StdIoWrapper::from(storage);
        let opts = fatfs::FormatVolumeOptions::new()
            .fat_type(fat_type)
            .hidden_sectors(2048);
        fatfs::format_volume(&mut stream, opts).expect("format volume");
        let disk = stream.into_inner().into_inner();
        // BPB_HiddSec, at offset 0x1C of the boot sector (and of its backup copy on FAT32)
        assert_eq!(disk[0x1C..0x20], 2048_u32.to_le_bytes());
        if fat_type == FatType::Fat32 {
            assert_eq!(disk[6 * 512 + 0x1C..6 * 512 + 0x20], 2048_u32.to_le_bytes());
        }
        let fs = fatfs::FileSystem::new(std::io::Cursor::new(disk), fatfs::FsOptions::new()).expect("open fs");
        assert_eq!(fs.fat_type(), fat_type);
    }
}

#[test]
fn test_format_reserved_sectors() {
    init_logger();
    for (fat_type, reserved_sectors) in [(FatType::Fat16, 4_u16), (FatType::Fat32, 32)] {
        let storage = std::io::Cursor::new(vec![0_u8; 64 * MB as usize]);
        let mut stream = fatfs::StdIoWrapper::from(storage);
        let opts = fatfs::FormatVolumeOptions::new()
            .fat_type(fat_type)
            .reserved_sectors(reserved_sectors);
        fatfs::format_volume(&mut stream, opts).expect("format volume");
        let disk = stream.into_inner().into_inner();
        // BPB_RsvdSecCnt, at offset 0x0E of the boot sector (and of its backup copy on FAT32)
        assert_eq!(disk[0x0E..0x10], reserved_sectors.to_le_bytes());
        if fat_type == FatType::Fat32 {
            assert_eq!(disk[6 * 512 + 0x0E..6 * 512 + 0x10], reserved_sectors.to_le_bytes());
        }
        // the first FAT starts right after them, with the media byte in its first entry
        assert_eq!(disk[usize::from(reserved_sectors) * 512], 0xF8);
        let fs = fatfs::FileSystem::new(std::io::Cursor::new(disk), fatfs::FsOptions::new()).expect("open fs");
        assert_eq!(fs.fat_type(), fat_type);
        let mut file = fs.root_dir().create_file("test.txt").expect("create file");
        file.write_all(TEST_STR.as_bytes()).expect("write file");
    }
}

#[test]
fn test_format_too_few_reserved_sectors() {
    init_logger();
    let storage = std::io::Cursor::new(vec![0_u8; 64 * MB as usize]);
    let mut stream = fatfs::StdIoWrapper::from(storage);
    let opts = fatfs::FormatVolumeOptions::new()
        .fat_type(FatType::Fat32)
        .reserved_sectors(4);
    assert!(fatfs::format_volume(&mut stream, opts).is_err());
}
