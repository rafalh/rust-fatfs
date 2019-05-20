Rust FAT FS
===========

[![Travis Build Status](https://travis-ci.org/rafalh/rust-fatfs.svg?branch=master)](https://travis-ci.org/rafalh/rust-fatfs)
[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE.txt)
[![crates.io](http://meritbadge.herokuapp.com/fatfs)](https://crates.io/crates/fatfs)
[![Documentation](https://docs.rs/fatfs/badge.svg)](https://docs.rs/fatfs)
[![Minimum rustc version](https://img.shields.io/badge/rustc-1.24+-yellow.svg)](https://blog.rust-lang.org/2018/02/15/Rust-1.24.html)

A FAT filesystem library implemented in Rust.

Features:
* read/write file using standard Read/Write traits
* read directory contents
* create/remove file or directory
* rename/move file or directory
* read/write file timestamps (updated automatically if `chrono` feature is enabled)
* format volume
* FAT12, FAT16, FAT32 compatibility
* LFN (Long File Names) extension is supported
* Basic no_std environment support

Usage
-----

Add this to your `Cargo.toml`:

    [dependencies]
    fatfs = "0.3"

and this to your crate root:

    extern crate fatfs;

You can start using the `fatfs` library now:

    let img_file = File::open("fat.img")?;
    let fs = fatfs::FileSystem::new(img_file, fatfs::FsOptions::new())?;
    let root_dir = fs.root_dir();
    let mut file = root_dir.create_file("hello.txt")?;
    file.write_all(b"Hello World!")?;

Note: it is recommended to wrap the underlying file struct in a buffering/caching object like `BufStream` from `fscommon` crate. For example:

    extern crate fscommon;
    let buf_stream = BufStream::new(img_file);
    let fs = fatfs::FileSystem::new(buf_stream, fatfs::FsOptions::new())?;

See more examples in the `examples` subdirectory.

no_std usage
------------

Add this to your `Cargo.toml`:

    [dependencies]
    fatfs = { version = "0.3", features = ["core_io"], default-features = false }

Note: LFN support requires `alloc` and `core_io/collections` features and makes use of `alloc` crate.
You may have to provide a memory allocator implementation.

For building in `no_std` mode a nightly Rust compiler version compatible with the current `core_io` crate is required.
See a date string in the `core_io` dependency version.

License
-------
The MIT license. See LICENSE.txt.
