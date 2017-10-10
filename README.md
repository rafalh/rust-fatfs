Rust FAT
========

[![Travis Build Status](https://travis-ci.org/rafalh/rust-fat.svg?branch=master)](https://travis-ci.org/rafalh/rust-fat)
[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE.txt)
[![crates.io](http://meritbadge.herokuapp.com/fatfs)](https://crates.io/crates/fatfs)
[![Documentation](https://docs.rs/fatfs/badge.svg)](https://docs.rs/fatfs)

Introduction
------------

FAT filesystem library implemented in Rust.

Features:
* FAT12, FAT16, FAT32 supported,
* read directory entries,
* read file contents,
* LFN (Long File Names),
* basic write support (write and truncate existing file).

Missing features (yet):
* create new file/directory,
* remove file/directory,

Other planned features (Nice to Have):
* no_std environment support.

License
-------
The MIT license. See LICENSE.txt.
