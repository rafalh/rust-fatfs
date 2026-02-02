#![cfg(target_os = "linux")]
use fatfs::Write;

const KB: u32 = 1024;
const MB: u32 = KB * 1024;

#[test]
fn test_fsck_1mb() {
    let _ = env_logger::builder().is_test(true).try_init();

    let mut image = std::fs::OpenOptions::new()
        .write(true)
        .read(true)
        .create(true)
        .open("/tmp/test.img")
        .expect("open temporary image file");
    image.set_len(MB as u64).expect("set_len on temp file");

    fatfs::format_volume(
        &mut fatfs::StdIoWrapper::from(image.try_clone().expect("clone tempfile")),
        fatfs::FormatVolumeOptions::new().total_sectors(MB / 512),
    )
    .expect("format volume");

    let fsck_status = std::process::Command::new("fsck.vfat")
        .args(&["-y", "/tmp/test.img"])
        .spawn()
        .expect("spawn fsck")
        .wait()
        .expect("wait on fsck");
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
        .args(&["-y", "/tmp/test.img"])
        .spawn()
        .expect("spawn fsck")
        .wait()
        .expect("wait on fsck");
    assert!(fsck_status.success(), "fsck was not successful ({fsck_status:?})");
}

#[test]
fn test_fsck_33mb_fat32() {
    let _ = env_logger::builder().is_test(true).try_init();

    let disk_img = "/tmp/test-fat32.img";

    let mut image = std::fs::OpenOptions::new()
        .write(true)
        .read(true)
        .create(true)
        .open(disk_img)
        .expect("open temporary image file");
    let len = 33 * MB;
    image.set_len(len as u64).expect("set_len on temp file");

    fatfs::format_volume(
        &mut fatfs::StdIoWrapper::from(image.try_clone().expect("clone tempfile")),
        fatfs::FormatVolumeOptions::new().fat_type(fatfs::FatType::Fat32),
    )
    .expect("format volume");

    let fsck_status = std::process::Command::new("fsck.vfat")
        .args(&["-y", disk_img])
        .spawn()
        .expect("spawn fsck")
        .wait()
        .expect("wait on fsck");
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
        .args(&["-y", disk_img])
        .spawn()
        .expect("spawn fsck")
        .wait()
        .expect("wait on fsck");
    assert!(fsck_status.success(), "fsck was not successful ({fsck_status:?})");
}
