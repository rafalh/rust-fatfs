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
//! ```rust
//! // Declare external crates
//! // Note: `fscommon` crate is used to speedup IO operations
//! extern crate fatfs;
//! extern crate fscommon;
//!
//! use std::io::prelude::*;
//!
//! fn main() -> std::io::Result<()> {
//!     # std::fs::copy("resources/fat16.img", "tmp/fat.img")?;
//!     // Initialize a filesystem object
//!     let img_file = std::fs::OpenOptions::new().read(true).write(true)
//!         .open("tmp/fat.img")?;
//!     let buf_stream = fscommon::BufStream::new(img_file);
//!     let fs = fatfs::FileSystem::new(buf_stream, fatfs::FsOptions::new())?;
//!     let root_dir = fs.root_dir();
//!
//!     // Write a file
//!     root_dir.create_dir("foo")?;
//!     let mut file = root_dir.create_file("foo/hello.txt")?;
//!     file.truncate()?;
//!     file.write_all(b"Hello World!")?;
//!
//!     // Read a directory
//!     let dir = root_dir.open_dir("foo")?;
//!     for r in dir.iter() {
//!         let entry = r?;
//!         println!("{}", entry.file_name());
//!     }
//!     # std::fs::remove_file("tmp/fat.img")?;
//!     # Ok(())
//! }
//! ```

#![crate_type = "lib"]
#![crate_name = "fatfs"]
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(all(not(feature = "std"), feature = "alloc"), feature(alloc))]
// Disable warnings to not clutter code with cfg too much
#![cfg_attr(not(all(feature = "alloc", feature = "lfn")), allow(dead_code, unused_imports))]
// `alloc` compiler feature is needed in Rust before 1.36
#![cfg_attr(all(not(feature = "std"), feature = "alloc"), allow(stable_features))]

#[macro_use]
extern crate bitflags;

#[macro_use]
extern crate log;

#[cfg(not(feature = "std"))]
extern crate core;

#[cfg(all(not(feature = "std"), feature = "alloc"))]
extern crate alloc;

mod boot_sector;
mod dir;
mod dir_entry;
mod file;
mod fs;
mod table;
mod time;
mod io;

mod byteorder_core_io;

use crate::byteorder_core_io as byteorder_ext;

#[cfg(feature = "std")]
use std as core;

pub use crate::dir::*;
pub use crate::dir_entry::*;
pub use crate::file::*;
pub use crate::fs::*;
pub use crate::time::*;
pub use crate::io::*;
