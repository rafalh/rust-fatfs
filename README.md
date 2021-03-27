Rust FAT FS
===========

[![CI Status](https://github.com/rafalh/rust-fatfs/actions/workflows/ci.yml/badge.svg)](https://github.com/rafalh/rust-fatfs/actions/workflows/ci.yml)
[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE.txt)
[![crates.io](https://img.shields.io/crates/v/fatfs)](https://crates.io/crates/fatfs)
[![Documentation](https://docs.rs/fatfs/badge.svg)](https://docs.rs/fatfs)
[![Minimum rustc version](https://img.shields.io/badge/rustc-1.36+-yellow.svg)](https://blog.rust-lang.org/2019/07/04/Rust-1.36.0.html)

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
    fatfs = "0.4"

and this to your crate root:

    extern crate fatfs;

You can start using the `fatfs` library now:

    let img_file = File::open("fat.img")?;
    let fs = fatfs::FileSystem::new(img_file, fatfs::FsOptions::new())?;
    let root_dir = fs.root_dir();
    let mut file = root_dir.create_file("hello.txt")?;
    file.write_all(b"Hello World!")?;

Note: it is recommended to wrap the underlying file struct in a buffering/caching object like `BufStream` from
`fscommon` crate. For example:

    extern crate fscommon;
    let buf_stream = BufStream::new(img_file);
    let fs = fatfs::FileSystem::new(buf_stream, fatfs::FsOptions::new())?;

See more examples in the `examples` subdirectory.

no_std usage
------------

Add this to your `Cargo.toml`:

    [dependencies]
    fatfs = { version = "0.4", default-features = false }

Additional features:

* `lfn` - LFN (long file name) support
* `alloc` - use `alloc` crate for dynamic allocation. Needed for API which uses `String` type. You may have to provide
a memory allocator implementation.
* `unicode` - use Unicode-compatible case conversion in file names - you may want to have it disabled for lower memory
footprint

Note: above features are enabled by default and were designed primarily for `no_std` usage.

License
-------
The MIT license. See `LICENSE.txt`.
