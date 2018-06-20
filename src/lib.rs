//! A FAT filesystem library implemented in Rust.
//!
//! # Usage
//!
//! This crate is [on crates.io](https://crates.io/crates/fatfs) and can be
//! used by adding `fatfs` to the dependencies in your project's `Cargo.toml`.
//!
//! ```toml
//! [dependencies]
//! fatfs = "0.3"
//! ```
//!
//! And this in your crate root:
//!
//! ```rust
//! extern crate fatfs;
//! ```
//!
//! # Examples
//!
//! Initialize a filesystem object (note: `fscommon` crate is used to speedup IO operations):
//! ```rust
//! let img_file = File::open("fat.img")?;
//! let buf_stream = fscommon::BufStream::new(img_file);
//! let fs = fatfs::FileSystem::new(buf_stream, fatfs::FsOptions::new())?;
//! ```
//! Write a file:
//! ```rust
//! let root_dir = fs.root_dir();
//! root_dir.create_dir("foo/bar")?;
//! let mut file = root_dir.create_file("foo/bar/hello.txt")?;
//! file.truncate()?;
//! file.write_all(b"Hello World!")?;
//! ```
//! Read a directory:
//! ```rust
//! let root_dir = fs.root_dir();
//! let dir = root_dir.open_dir("foo/bar")?;
//! for r in dir.iter()? {
//!     let entry = r?;
//!     println!(entry.file_name());
//! }
//! ```

#![crate_type = "lib"]
#![crate_name = "fatfs"]

#![cfg_attr(not(feature="std"), no_std)]
#![cfg_attr(not(feature="std"), feature(alloc))]

// Disable warnings to not clutter code with cfg too much
#![cfg_attr(not(feature="alloc"), allow(dead_code, unused_imports))]

extern crate byteorder;

#[macro_use]
extern crate bitflags;

#[macro_use]
extern crate log;

#[cfg(feature = "chrono")]
extern crate chrono;

#[cfg(not(feature = "std"))]
extern crate core_io;

#[cfg(all(not(feature = "std"), feature = "alloc"))]
extern crate alloc;

mod fs;
mod dir;
mod dir_entry;
mod file;
mod table;

#[cfg(not(feature = "std"))]
mod byteorder_core_io;

#[cfg(not(feature = "std"))]
use byteorder_core_io as byteorder_ext;
#[cfg(feature = "std")]
use byteorder as byteorder_ext;
#[cfg(feature = "std")]
use std as core;
#[cfg(not(feature = "std"))]
use core_io as io;

#[cfg(feature = "std")]
use std::io as io;

pub use fs::*;
pub use dir::*;
pub use dir_entry::*;
pub use file::*;
