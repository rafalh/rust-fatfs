TODO
====
* proper support for short name decoding from the OEM codepage
* marking volume dirty on first write and not-dirty on unmount
* support for a volume label file in the root directory
* format volume API
* add method for getting `DirEntry` from a path (possible names: metadata, lookup)
* add time provier so the crate writes a proper timestamps when `chrono` is unavailable
* do not create LFN entries if the name fits in a SFN entry
