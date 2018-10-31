extern crate env_logger;
extern crate fatfs;
extern crate fscommon;

use std::io;
use std::io::prelude::*;

use fscommon::BufStream;

const KB: u32 = 1024;
const MB: u32 = KB * 1024;
const TEST_STR: &str = "Hi there Rust programmer!\n";

#[test]
fn test_format() {
    let _ = env_logger::try_init();
    let storage_vec: Vec<u8> = Vec::with_capacity((8 * MB) as usize);
    let storage_cur = io::Cursor::new(storage_vec);
    let mut buffered_stream = BufStream::new(storage_cur);
    let mut opts: fatfs::FormatOptions = Default::default();
    opts.total_sectors = 8 * MB / 512;
    fatfs::format_volume(&mut buffered_stream, opts).expect("format volume");

    let fs = fatfs::FileSystem::new(buffered_stream, fatfs::FsOptions::new()).expect("open fs");
    let root_dir = fs.root_dir();
    let entries = root_dir.iter().map(|r| r.unwrap()).collect::<Vec<_>>();
    assert_eq!(entries.len(), 0);

    let mut file = root_dir.create_file("short.txt").expect("create file");
    file.truncate().expect("truncate file");
    file.write_all(&TEST_STR.as_bytes()).expect("write file");

    let entries = root_dir.iter().map(|r| r.unwrap()).collect::<Vec<_>>();
    assert_eq!(entries.len(), 1);
}
