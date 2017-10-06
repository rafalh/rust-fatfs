#![crate_type = "lib"]
#![crate_name = "rfat"]

extern crate byteorder;
extern crate core;

#[macro_use]
extern crate bitflags;

#[cfg(feature = "chrono")]
extern crate chrono;

mod fs;
mod dir;
mod file;
mod table;

pub use fs::*;
pub use dir::*;
pub use file::*;
