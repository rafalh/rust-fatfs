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
