#![allow(dead_code)]

use std::path::Path;

use fatfs::StdIoWrapper;
use fscommon::BufStream;
use tempfile::TempPath;

pub const KB: u64 = 1024;
pub const MB: u64 = KB * 1024;

pub const FAT12_IMG: &str = "fat12.img";
pub const FAT16_IMG: &str = "fat16.img";
pub const FAT32_IMG: &str = "fat32.img";
pub const IMG_DIR: &str = "resources";

pub type FileBackedFs = fatfs::FileSystem<StdIoWrapper<BufStream<std::fs::File>>>;
pub type MemBackedFs = fatfs::FileSystem<StdIoWrapper<BufStream<std::io::Cursor<Vec<u8>>>>>;

pub fn init_logger() {
    let _ = env_logger::builder().is_test(true).try_init();
}

pub fn open_fs_ro(filename: &str) -> FileBackedFs {
    let file = std::fs::File::open(format!("{}/{}", IMG_DIR, filename)).unwrap();
    let buf_file = BufStream::new(file);
    fatfs::FileSystem::new(buf_file, fatfs::FsOptions::new()).unwrap()
}

pub fn copy_fs_img(filename: &str) -> TempPath {
    let src_path = format!("{}/{}", IMG_DIR, filename);
    let tmp = tempfile::NamedTempFile::with_suffix("fat.img").expect("create named tempfile");
    let tmp_path = tmp.into_temp_path();
    std::fs::copy(src_path, &tmp_path).expect("copy image");
    tmp_path
}

pub fn open_fs_rw(path: &Path) -> FileBackedFs {
    let file = std::fs::OpenOptions::new().read(true).write(true).open(path).unwrap();
    let buf_file = BufStream::new(file);
    let fs = fatfs::FileSystem::new(buf_file, fatfs::FsOptions::new()).unwrap();
    fs
}

pub fn create_temp_fs(opts: fatfs::FormatVolumeOptions, total_bytes: u64) -> MemBackedFs {
    // Init storage to 0xD1 bytes (value has been choosen to be parsed as normal file)
    let storage_vec: Vec<u8> = vec![0xD1_u8; total_bytes as usize];
    let storage_cur = std::io::Cursor::new(storage_vec);
    let mut buffered_stream = fatfs::StdIoWrapper::from(BufStream::new(storage_cur));
    fatfs::format_volume(&mut buffered_stream, opts).expect("format volume");
    fatfs::FileSystem::new(buffered_stream, fatfs::FsOptions::new()).expect("open fs")
}

pub fn create_temp_file_fs(opts: fatfs::FormatVolumeOptions, total_bytes: u64) -> (FileBackedFs, TempPath) {
    let tmp = tempfile::NamedTempFile::with_suffix("fat.img").expect("create named tempfile");
    let (img_file, path) = tmp.into_parts();

    img_file.set_len(total_bytes).expect("set_len on temp file");

    fatfs::format_volume(
        &mut fatfs::StdIoWrapper::from(BufStream::new(img_file.try_clone().unwrap())),
        opts,
    )
    .expect("format volume");

    let fs = fatfs::FileSystem::new(BufStream::new(img_file), fatfs::FsOptions::new()).expect("open fs");
    (fs, path)
}
