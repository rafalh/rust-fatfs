extern crate fatfs;
extern crate fscommon;

use std::env;
use std::fs;
use std::io;

use fscommon::BufStream;

fn main() -> io::Result<()> {
    let filename = env::args().nth(1).expect("image path expected");
    let file = fs::OpenOptions::new().read(true).write(true).open(&filename)?;
    let buf_file = BufStream::new(file);
    fatfs::format_volume(buf_file, fatfs::FormatVolumeOptions::new())?;
    Ok(())
}
