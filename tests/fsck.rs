use std::process::{Command, Stdio};

use fatfs::{FatType, Write};

const KB: u32 = 1024;
const MB: u32 = KB * 1024;

#[cfg(target_os = "macos")]
fn find_fsck_command(img_path: &str) -> Option<Command> {
    let fsck_names = &["fsck.vfat", "fsck_msdos"];
    for name in fsck_names {
        if Command::new(name).stdout(Stdio::null()).status().is_ok() {
            let mut cmd = Command::new(name);
            cmd.args(&["-n", &img_path]);
            return Some(cmd);
        }
    }
    None
}

fn create_and_fsck_image(fat_type: FatType, size: u32) {
    let mut disk_img =
        tempfile::NamedTempFile::with_suffix(format!("-{fat_type:?}-{size}.img")).expect("create named tempfile");
    let disk_img_path = disk_img.path().display().to_string();

    let Some(mut fsck_command) = find_fsck_command(&disk_img_path) else {
        log::warn!("fsck not available");
        return;
    };

    let image = disk_img.as_file_mut();
    image.set_len(size as u64).expect("set_len on temp file");

    fatfs::format_volume(
        &mut fatfs::StdIoWrapper::from(image.try_clone().expect("clone tempfile")),
        fatfs::FormatVolumeOptions::new()
            .fat_type(fat_type)
            .total_sectors(size / 512),
    )
    .expect("format volume");

    let fsck_status = fsck_command.status().unwrap();
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

    let fsck_status = fsck_command.status().unwrap();
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
