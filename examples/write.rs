extern crate fatfs;

use std::fs::OpenOptions;
use std::io::prelude::*;

use fatfs::{FileSystem, FsOptions, BufStream};

fn main() {
    let img_file = match OpenOptions::new().read(true).write(true).open("fat.img") {
        Ok(file) => file,
        Err(err) => {
            println!("Failed to open image: {}", err);
            return;
        }
    };
    let mut buf_stream = BufStream::new(img_file);
    let options = FsOptions::new().update_accessed_date(true);
    let fs = FileSystem::new(&mut buf_stream, options).unwrap();
    let mut file = fs.root_dir().create_file("hello.txt").unwrap();
    file.write_all(b"Hello World!").unwrap();
}
