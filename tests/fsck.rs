mod common;

use std::{
    path::Path,
    process::{Command, Stdio},
};

use fatfs::{FatType, Write};

use crate::common::{create_temp_file_fs, init_logger, KB, MB};

fn find_fsck_command(img_path: &Path) -> Option<Command> {
    let fsck_names = &["fsck.vfat", "fsck_msdos"];
    for name in fsck_names {
        if Command::new(name).stdout(Stdio::null()).status().is_ok() {
            let mut cmd = Command::new(name);
            cmd.arg("-n").arg(img_path);
            return Some(cmd);
        }
    }
    None
}

fn create_and_fsck_image(fat_type: FatType, size: u64) {
    let opts = fatfs::FormatVolumeOptions::new()
        .fat_type(fat_type)
        .total_sectors((size / 512).try_into().unwrap());
    let (fs, img_path) = create_temp_file_fs(opts, size);

    let Some(mut fsck_command) = find_fsck_command(img_path.as_ref()) else {
        log::warn!("fsck not available");
        return;
    };

    let fsck_status = fsck_command.status().unwrap();
    assert!(fsck_status.success(), "fsck was not successful ({fsck_status:?})");

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
    init_logger();
    create_and_fsck_image(FatType::Fat12, MB);
}

#[test]
fn test_fsck_33mb_fat16() {
    init_logger();
    create_and_fsck_image(FatType::Fat16, 33 * MB);
}

#[test]
fn test_fsck_33mb_fat32() {
    init_logger();
    create_and_fsck_image(FatType::Fat32, 33 * MB);
}
