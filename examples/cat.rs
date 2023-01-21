use std::env;
use std::fs::File;
use std::io::{self, prelude::*};

use fatfs::{FsOptions, StdIoWrapper, DefaultTimeProvider, LossyOemCpConverter};
use fscommon::BufStream;

type FileSystem =
    fatfs::FileSystem<StdIoWrapper<BufStream<File>>, DefaultTimeProvider, LossyOemCpConverter>;

fn main() -> io::Result<()> {
    let file = File::open("resources/fat32.img")?;
    let buf_rdr = BufStream::new(file);
    let fs = FileSystem::new(buf_rdr.into(), FsOptions::new())?;
    let root_dir = fs.root_dir();
    let mut file = root_dir.open_file(&env::args().nth(1).expect("filename expected"))?;
    let mut buf = vec![];
    file.read_to_end(&mut buf)?;
    print!("{}", String::from_utf8_lossy(&buf));
    Ok(())
}
