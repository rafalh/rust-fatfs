Changelog
=========

0.3.6 (2023-01-17)
------------------------
Bug fixes:
* Create directory entry with `VOLUME_ID` attribute when formatting if volume label was set in `FormatVolumeOptions`
* Fix `format_volume` function panicking in debug build for FAT12 volumes with size below 1 MB

0.3.5 (2021-01-23)
------------------------
Bug fixes:
* Fix file-system corruption that occurs when creating multiple directory entries in a non-root directory (directory
  size must be greater or equal to the cluster size for the corruption to happen)

0.3.4 (2020-07-20)
------------------
Bug fixes:
* Fix time encoding and decoding in directory entries

0.3.3 (2019-11-10)
------------------
Bug fixes:
* Add missing characters to the whitelist for long file name (`^`, `#`, `&`)
* Fix invalid short file names for `.` and `..` entries when creating a new directory
* Fix `no_std` build

Misc changes:
* Fix compiler warnings
* Improve documentation

0.3.2 (2018-12-29)
------------------
New features:
* Add `format_volume` function for initializing a FAT filesystem on a partition
* Add more checks of filesystem correctness when mounting

Bug fixes:
* Clear directory returned from `create_dir` method - upgrade ASAP if this method is used
* Fix handling of FSInfo sector on FAT32 volumes with sector size different than 512 - upgrade ASAP if such sector size is used
* Use `write_all` in `serialize` method for FSInfo sector - previously it could have been improperly updated

0.3.1 (2018-10-20)
------------------
New features:
* Increased file creation time resolution from 2s to 1/100s
* Added oem_cp_converter filesystem option allowing to provide custom short name decoder
* Added time_provider filesystem option allowing to provide time used when modifying directory entries
* Added marking volume as dirty on first write and not-dirty on unmount
* Added support for reading volume label from root directory

Bug fixes:
* Fixed handling of short names with spaces in the middle - all characters after first space in 8.3 components were
  stripped before
* Fixed decoding 0xE5 character in first byte of short name - if first character of short name is equal to 0xE5,
  it was read as 0x05
* Preserve 4 most significant bits in FAT32 entries - it is required by FAT specification, but previous behavior
  should not cause any compatibility problem because all known implementations ignore those bits
* Fixed warnings for file entries without LFN entries - they were handled properly, but caused warnings in run-time

Misc changes:
* Deprecated set_created. set_accessed, set_modified methods on File - those fields are updated automatically using
  information provided by TimeProvider
* Fixed size formatting in ls.rs example
* Added more filesystem checks causing errors or warnings when incompatibility is detected
* Removed unnecessary clone() calls
* Code formatting and docs fixes
