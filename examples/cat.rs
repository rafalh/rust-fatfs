extern crate fatfs;

use std::env;
use std::fs::File;
use std::io::BufReader;
use std::io::prelude::*;
use std::str;

use fatfs::FileSystem;

fn main() {
    let file = File::open("resources/fat32.img").unwrap();
    let mut buf_rdr = BufReader::new(file);
    let fs = FileSystem::new(&mut buf_rdr).unwrap();
    let mut root_dir = fs.root_dir();
    let mut file = root_dir.open_file(&env::args().nth(1).unwrap()).unwrap();
    let mut buf = vec![];
    file.read_to_end(&mut buf).unwrap();
    print!("{}", str::from_utf8(&buf).unwrap());
}
