use std::fs::OpenOptions;
use std::io::{self, prelude::*};

use fatfs::{FileSystem, FsOptions, StdIoWrapper};
use fscommon::BufStream;

fn main() -> io::Result<()> {
    let img_file = match OpenOptions::new().create(true).read(true).write(true).open("fat.img") {
        Ok(file) => file,
        Err(err) => {
            println!("Failed to open image!");
            return Err(err);
        }
    };
    let buf_stream = BufStream::new(img_file);
    let options = FsOptions::new().update_accessed_date(true);
    let mut disk = StdIoWrapper::new(buf_stream);
    let fs = FileSystem::new(&mut disk, options).unwrap();
    let mut file = fs.root_dir().create_file("hello.txt")?;
    file.write_all(b"Hello World!")?;
    Ok(())
}
