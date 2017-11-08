Rust FAT FS
===========

[![Travis Build Status](https://travis-ci.org/rafalh/rust-fat.svg?branch=master)](https://travis-ci.org/rafalh/rust-fat)
[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE.txt)
[![crates.io](http://meritbadge.herokuapp.com/fatfs)](https://crates.io/crates/fatfs)
[![Documentation](https://docs.rs/fatfs/badge.svg)](https://docs.rs/fatfs)

FAT filesystem library implemented in Rust.

Features:
* read/write/create/remove file,
* enumerate directory children,
* create/remove directory,
* read/write file timestamps (updated automatically if chrono is available),
* FAT12, FAT16, FAT32 compatibility,
* LFN (Long File Names) extension supported.

Usage
-----

Put this in your `Cargo.toml`:

    [dependencies]
    fatfs = "0.1"

Put this in your crate root:

    extern crate fatfs;

You can start using library now:

    let img_file = File::open("fat.img").unwrap();
    let mut buf_stream = BufStream::new(img_file);
    let fs = fatfs::FileSystem::new(&mut buf_stream, true).unwrap();
    let mut root_dir = fs.root_dir();
    let mut file = root_dir.create_file("hello.txt").unwrap();
    file.write_all(b"Hello World!").unwrap();

See more examples in `examples` subdirectory.

License
-------
The MIT license. See LICENSE.txt.
