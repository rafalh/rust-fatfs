extern crate fatfs;

use std::env;
use std::fs::File;
use std::io::{self, prelude::*};

use fatfs::{FileSystem, FsOptions, BufStream};

fn main() -> io::Result<()> {
    let file = File::open("resources/fat32.img")?;
    let mut buf_rdr = BufStream::new(file);
    let fs = FileSystem::new(&mut buf_rdr, FsOptions::new())?;
    let mut root_dir = fs.root_dir();
    let mut file = root_dir.open_file(&env::args().nth(1).unwrap())?;
    let mut buf = vec![];
    file.read_to_end(&mut buf)?;
    print!("{}", String::from_utf8_lossy(&buf));
    Ok(())
}
