#![cfg(target_os = "linux")]

use fatfs::{FatType, Write};

const KB: u32 = 1024;
const MB: u32 = KB * 1024;

fn create_and_fsck_image(fat_type: FatType, size: u32) {
    let mut disk_img =
        tempfile::NamedTempFile::with_suffix(format!("-{fat_type:?}-{size}.img")).expect("create named tempfile");
    let disk_img_path = disk_img.path().display().to_string();

    let image = disk_img.as_file_mut();
    image.set_len(size as u64).expect("set_len on temp file");

    fatfs::format_volume(
        &mut fatfs::StdIoWrapper::from(image.try_clone().expect("clone tempfile")),
        fatfs::FormatVolumeOptions::new()
            .fat_type(fat_type)
            .total_sectors(size / 512),
    )
    .expect("format volume");

    let fsck_status = std::process::Command::new("fsck.vfat")
        .args(&["-y", &disk_img_path])
        .status()
        .expect("get fsck.vfat status");
    assert!(fsck_status.success(), "fsck was not successful ({fsck_status:?})");

    let fs = fatfs::FileSystem::new(image, fatfs::FsOptions::new()).expect("open fs");
    fs.root_dir().create_dir("dir1").expect("create dir1");
    fs.root_dir()
        .create_file("root file.bin")
        .expect("create root file")
        .write_all(&[0xab; (16 * KB) as usize])
        .expect("root file write");
    let dir2 = fs.root_dir().create_dir("dir2").expect("create dir2");
    dir2.create_dir("subdir").expect("subdir");
    dir2.create_file("file1")
        .expect("file1")
        .write_all(b"testing 1 2 1 2")
        .expect("file 1 write");
    core::mem::drop(dir2);
    core::mem::drop(fs);

    let fsck_status = std::process::Command::new("fsck.vfat")
        .args(&["-y", &disk_img_path])
        .status()
        .expect("get fsck.vfat status");
    assert!(fsck_status.success(), "fsck was not successful ({fsck_status:?})");
}

#[test]
fn test_fsck_1mb_fat12() {
    let _ = env_logger::builder().is_test(true).try_init();

    create_and_fsck_image(FatType::Fat12, MB);
}

#[test]
fn test_fsck_33mb_fat16() {
    let _ = env_logger::builder().is_test(true).try_init();

    create_and_fsck_image(FatType::Fat16, 33 * MB);
}

#[test]
fn test_fsck_33mb_fat32() {
    let _ = env_logger::builder().is_test(true).try_init();

    create_and_fsck_image(FatType::Fat32, 33 * MB);
}
