Changelog
=========

Version 0.3.1
-------------
New features:
* Increased file creation time resolution from 2s to 1/100s
* Added oem_cp_converter filesystem option allowing to provide custom short name decoder
* Added time_provider filesystem option allowing to provide time used when modifying directory entries
* Added marking volume as dirty on first write and not-dirty on unmount
* Added support for reading volume label from root directory

Fixes:
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
