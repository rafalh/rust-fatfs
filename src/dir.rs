#[cfg(all(not(feature = "std"), feature = "alloc"))]
use alloc::vec::Vec;
use core::{char, cmp, num, str};
#[cfg(feature = "alloc")]
use core::{iter, slice};
use io;
use io::prelude::*;
use io::{ErrorKind, SeekFrom};

use dir_entry::{DirEntry, DirEntryData, DirFileEntryData, DirLfnEntryData, FileAttributes, ShortName, DIR_ENTRY_SIZE};
#[cfg(feature = "alloc")]
use dir_entry::{LFN_ENTRY_LAST_FLAG, LFN_PART_LEN};
use file::File;
use fs::{DiskSlice, FileSystem, FsIoAdapter, ReadWriteSeek};

#[cfg(feature = "alloc")]
type LfnUtf16 = Vec<u16>;
#[cfg(not(feature = "alloc"))]
type LfnUtf16 = ();

const SFN_PADDING: u8 = 0x20;

pub(crate) enum DirRawStream<'a, T: ReadWriteSeek + 'a> {
    File(File<'a, T>),
    Root(DiskSlice<FsIoAdapter<'a, T>>),
}

impl<'a, T: ReadWriteSeek> DirRawStream<'a, T> {
    fn abs_pos(&self) -> Option<u64> {
        match self {
            &DirRawStream::File(ref file) => file.abs_pos(),
            &DirRawStream::Root(ref slice) => Some(slice.abs_pos()),
        }
    }

    fn first_cluster(&self) -> Option<u32> {
        match self {
            &DirRawStream::File(ref file) => file.first_cluster(),
            &DirRawStream::Root(_) => None,
        }
    }
}

// Note: derive cannot be used because of invalid bounds. See: https://github.com/rust-lang/rust/issues/26925
impl<'a, T: ReadWriteSeek> Clone for DirRawStream<'a, T> {
    fn clone(&self) -> Self {
        match self {
            &DirRawStream::File(ref file) => DirRawStream::File(file.clone()),
            &DirRawStream::Root(ref raw) => DirRawStream::Root(raw.clone()),
        }
    }
}

impl<'a, T: ReadWriteSeek> Read for DirRawStream<'a, T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            &mut DirRawStream::File(ref mut file) => file.read(buf),
            &mut DirRawStream::Root(ref mut raw) => raw.read(buf),
        }
    }
}

impl<'a, T: ReadWriteSeek> Write for DirRawStream<'a, T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            &mut DirRawStream::File(ref mut file) => file.write(buf),
            &mut DirRawStream::Root(ref mut raw) => raw.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            &mut DirRawStream::File(ref mut file) => file.flush(),
            &mut DirRawStream::Root(ref mut raw) => raw.flush(),
        }
    }
}

impl<'a, T: ReadWriteSeek> Seek for DirRawStream<'a, T> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match self {
            &mut DirRawStream::File(ref mut file) => file.seek(pos),
            &mut DirRawStream::Root(ref mut raw) => raw.seek(pos),
        }
    }
}

fn split_path<'c>(path: &'c str) -> (&'c str, Option<&'c str>) {
    // remove trailing slash and split into 2 components - top-most parent and rest
    let mut path_split = path.trim_matches('/').splitn(2, '/');
    let comp = path_split.next().unwrap(); // SAFE: splitn always returns at least one element
    let rest_opt = path_split.next();
    (comp, rest_opt)
}

enum DirEntryOrShortName<'a, T: ReadWriteSeek + 'a> {
    DirEntry(DirEntry<'a, T>),
    ShortName([u8; 11]),
}

/// A FAT filesystem directory.
///
/// This struct is created by the `open_dir` or `create_dir` methods on `Dir`.
/// The root directory is returned by the `root_dir` method on `FileSystem`.
pub struct Dir<'a, T: ReadWriteSeek + 'a> {
    stream: DirRawStream<'a, T>,
    fs: &'a FileSystem<T>,
}

impl<'a, T: ReadWriteSeek + 'a> Dir<'a, T> {
    pub(crate) fn new(stream: DirRawStream<'a, T>, fs: &'a FileSystem<T>) -> Self {
        Dir { stream, fs }
    }

    /// Creates directory entries iterator.
    pub fn iter(&self) -> DirIter<'a, T> {
        DirIter::new(self.stream.clone(), self.fs, true)
    }

    fn find_entry(
        &self,
        name: &str,
        is_dir: Option<bool>,
        mut short_name_gen: Option<&mut ShortNameGenerator>,
    ) -> io::Result<DirEntry<'a, T>> {
        for r in self.iter() {
            let e = r?;
            // compare name ignoring case
            if e.eq_name(name) {
                // check if file or directory is expected
                if is_dir.is_some() && Some(e.is_dir()) != is_dir {
                    let error_msg = if e.is_dir() { "Is a directory" } else { "Not a directory" };
                    return Err(io::Error::new(ErrorKind::Other, error_msg));
                }
                return Ok(e);
            }
            // update short name generator state
            if let Some(ref mut gen) = short_name_gen {
                gen.add_existing(e.raw_short_name());
            }
        }
        Err(io::Error::new(ErrorKind::NotFound, "No such file or directory"))
    }

    pub(crate) fn find_volume_entry(&self) -> io::Result<Option<DirEntry<'a, T>>> {
        for r in DirIter::new(self.stream.clone(), self.fs, false) {
            let e = r?;
            if e.data.is_volume() {
                return Ok(Some(e));
            }
        }
        Ok(None)
    }

    fn check_for_existence(&self, name: &str, is_dir: Option<bool>) -> io::Result<DirEntryOrShortName<'a, T>> {
        let mut short_name_gen = ShortNameGenerator::new(name);
        loop {
            let r = self.find_entry(name, is_dir, Some(&mut short_name_gen));
            match r {
                Err(ref err) if err.kind() == ErrorKind::NotFound => {},
                // other error
                Err(err) => return Err(err),
                // directory already exists - return it
                Ok(e) => return Ok(DirEntryOrShortName::DirEntry(e)),
            };
            if let Ok(name) = short_name_gen.generate() {
                return Ok(DirEntryOrShortName::ShortName(name));
            }
            short_name_gen.next_iteration();
        }
    }

    /// Opens existing subdirectory.
    ///
    /// `path` is a '/' separated directory path relative to self directory.
    pub fn open_dir(&self, path: &str) -> io::Result<Self> {
        trace!("open_dir {}", path);
        let (name, rest_opt) = split_path(path);
        let e = self.find_entry(name, Some(true), None)?;
        match rest_opt {
            Some(rest) => e.to_dir().open_dir(rest),
            None => Ok(e.to_dir()),
        }
    }

    /// Opens existing file.
    ///
    /// `path` is a '/' separated file path relative to self directory.
    pub fn open_file(&self, path: &str) -> io::Result<File<'a, T>> {
        trace!("open_file {}", path);
        // traverse path
        let (name, rest_opt) = split_path(path);
        if let Some(rest) = rest_opt {
            let e = self.find_entry(name, Some(true), None)?;
            return e.to_dir().open_file(rest);
        }
        // convert entry to a file
        let e = self.find_entry(name, Some(false), None)?;
        Ok(e.to_file())
    }

    /// Creates new or opens existing file=.
    ///
    /// `path` is a '/' separated file path relative to self directory.
    /// File is never truncated when opening. It can be achieved by calling `File::truncate` method after opening.
    pub fn create_file(&self, path: &str) -> io::Result<File<'a, T>> {
        trace!("create_file {}", path);
        // traverse path
        let (name, rest_opt) = split_path(path);
        if let Some(rest) = rest_opt {
            return self.find_entry(name, Some(true), None)?.to_dir().create_file(rest);
        }
        // this is final filename in the path
        let r = self.check_for_existence(name, Some(false))?;
        match r {
            // file does not exist - create it
            DirEntryOrShortName::ShortName(short_name) => {
                let sfn_entry = self.create_sfn_entry(short_name, FileAttributes::from_bits_truncate(0), None);
                Ok(self.write_entry(name, sfn_entry)?.to_file())
            },
            // file already exists - return it
            DirEntryOrShortName::DirEntry(e) => Ok(e.to_file()),
        }
    }

    /// Creates new directory or opens existing.
    ///
    /// `path` is a '/' separated path relative to self directory.
    pub fn create_dir(&self, path: &str) -> io::Result<Self> {
        trace!("create_dir {}", path);
        // traverse path
        let (name, rest_opt) = split_path(path);
        if let Some(rest) = rest_opt {
            return self.find_entry(name, Some(true), None)?.to_dir().create_dir(rest);
        }
        // this is final filename in the path
        let r = self.check_for_existence(name, Some(true))?;
        match r {
            // directory does not exist - create it
            DirEntryOrShortName::ShortName(short_name) => {
                // alloc cluster for directory data
                let cluster = self.fs.alloc_cluster(None, true)?;
                // create entry in parent directory
                let sfn_entry = self.create_sfn_entry(short_name, FileAttributes::DIRECTORY, Some(cluster));
                let entry = self.write_entry(name, sfn_entry)?;
                let dir = entry.to_dir();
                // create special entries "." and ".."
                let dot_sfn = ShortNameGenerator::generate_dot();
                let sfn_entry = self.create_sfn_entry(dot_sfn, FileAttributes::DIRECTORY, entry.first_cluster());
                dir.write_entry(".", sfn_entry)?;
                let dotdot_sfn = ShortNameGenerator::generate_dotdot();
                let sfn_entry =
                    self.create_sfn_entry(dotdot_sfn, FileAttributes::DIRECTORY, self.stream.first_cluster());
                dir.write_entry("..", sfn_entry)?;
                Ok(dir)
            },
            // directory already exists - return it
            DirEntryOrShortName::DirEntry(e) => Ok(e.to_dir()),
        }
    }

    fn is_empty(&self) -> io::Result<bool> {
        trace!("is_empty");
        // check if directory contains no files
        for r in self.iter() {
            let e = r?;
            let name = e.short_file_name_as_bytes();
            // ignore special entries "." and ".."
            if name != b"." && name != b".." {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Removes existing file or directory.
    ///
    /// `path` is a '/' separated file path relative to self directory.
    /// Make sure there is no reference to this file (no File instance) or filesystem corruption
    /// can happen.
    pub fn remove(&self, path: &str) -> io::Result<()> {
        trace!("remove {}", path);
        // traverse path
        let (name, rest_opt) = split_path(path);
        if let Some(rest) = rest_opt {
            let e = self.find_entry(name, Some(true), None)?;
            return e.to_dir().remove(rest);
        }
        // in case of directory check if it is empty
        let e = self.find_entry(name, None, None)?;
        if e.is_dir() && !e.to_dir().is_empty()? {
            return Err(io::Error::new(ErrorKind::Other, "Directory not empty"));
        }
        // free data
        if let Some(n) = e.first_cluster() {
            self.fs.free_cluster_chain(n)?;
        }
        // free long and short name entries
        let mut stream = self.stream.clone();
        stream.seek(SeekFrom::Start(e.offset_range.0 as u64))?;
        let num = (e.offset_range.1 - e.offset_range.0) as usize / DIR_ENTRY_SIZE as usize;
        for _ in 0..num {
            let mut data = DirEntryData::deserialize(&mut stream)?;
            trace!("removing dir entry {:?}", data);
            data.set_deleted();
            stream.seek(SeekFrom::Current(-(DIR_ENTRY_SIZE as i64)))?;
            data.serialize(&mut stream)?;
        }
        Ok(())
    }

    /// Renames or moves existing file or directory.
    ///
    /// `src_path` is a '/' separated source file path relative to self directory.
    /// `dst_path` is a '/' separated destination file path relative to `dst_dir`.
    /// `dst_dir` can be set to self directory if rename operation without moving is needed.
    /// Make sure there is no reference to this file (no File instance) or filesystem corruption
    /// can happen.
    pub fn rename(&self, src_path: &str, dst_dir: &Dir<T>, dst_path: &str) -> io::Result<()> {
        trace!("rename {} {}", src_path, dst_path);
        // traverse source path
        let (name, rest_opt) = split_path(src_path);
        if let Some(rest) = rest_opt {
            let e = self.find_entry(name, Some(true), None)?;
            return e.to_dir().rename(rest, dst_dir, dst_path);
        }
        // traverse destination path
        let (name, rest_opt) = split_path(dst_path);
        if let Some(rest) = rest_opt {
            let e = dst_dir.find_entry(name, Some(true), None)?;
            return self.rename(src_path, &e.to_dir(), rest);
        }
        // move/rename file
        self.rename_internal(src_path, dst_dir, dst_path)
    }

    fn rename_internal(&self, src_name: &str, dst_dir: &Dir<T>, dst_name: &str) -> io::Result<()> {
        trace!("rename_internal {} {}", src_name, dst_name);
        // find existing file
        let e = self.find_entry(src_name, None, None)?;
        // check if destionation filename is unused
        let r = dst_dir.check_for_existence(dst_name, None)?;
        let short_name = match r {
            DirEntryOrShortName::DirEntry(ref dst_e) => {
                // check if source and destination entry is the same
                if e.is_same_entry(dst_e) {
                    return Ok(());
                }
                return Err(io::Error::new(ErrorKind::AlreadyExists, "Destination file already exists"));
            },
            DirEntryOrShortName::ShortName(short_name) => short_name,
        };
        // free long and short name entries
        let mut stream = self.stream.clone();
        stream.seek(SeekFrom::Start(e.offset_range.0 as u64))?;
        let num = (e.offset_range.1 - e.offset_range.0) as usize / DIR_ENTRY_SIZE as usize;
        for _ in 0..num {
            let mut data = DirEntryData::deserialize(&mut stream)?;
            trace!("removing LFN entry {:?}", data);
            data.set_deleted();
            stream.seek(SeekFrom::Current(-(DIR_ENTRY_SIZE as i64)))?;
            data.serialize(&mut stream)?;
        }
        // save new directory entry
        let sfn_entry = e.data.renamed(short_name);
        dst_dir.write_entry(dst_name, sfn_entry)?;
        Ok(())
    }

    fn find_free_entries(&self, num_entries: usize) -> io::Result<DirRawStream<'a, T>> {
        let mut stream = self.stream.clone();
        let mut first_free = 0;
        let mut num_free = 0;
        let mut i = 0;
        loop {
            let raw_entry = DirEntryData::deserialize(&mut stream)?;
            if raw_entry.is_end() {
                // first unused entry - all remaining space can be used
                if num_free == 0 {
                    first_free = i;
                }
                stream.seek(io::SeekFrom::Start(first_free as u64 * DIR_ENTRY_SIZE))?;
                return Ok(stream);
            } else if raw_entry.is_deleted() {
                // free entry - calculate number of free entries in a row
                if num_free == 0 {
                    first_free = i;
                }
                num_free += 1;
                if num_free == num_entries {
                    // enough space for new file
                    stream.seek(io::SeekFrom::Start(first_free as u64 * DIR_ENTRY_SIZE))?;
                    return Ok(stream);
                }
            } else {
                // used entry - start counting from 0
                num_free = 0;
            }
            i += 1;
        }
    }

    fn create_sfn_entry(
        &self,
        short_name: [u8; 11],
        attrs: FileAttributes,
        first_cluster: Option<u32>,
    ) -> DirFileEntryData {
        let mut raw_entry = DirFileEntryData::new(short_name, attrs);
        raw_entry.set_first_cluster(first_cluster, self.fs.fat_type());
        let now = self.fs.options.time_provider.get_current_date_time();
        raw_entry.set_created(now);
        raw_entry.set_accessed(now.date);
        raw_entry.set_modified(now);
        raw_entry
    }

    #[cfg(feature = "alloc")]
    fn encode_lfn_utf16(name: &str) -> Vec<u16> {
        name.encode_utf16().collect::<Vec<u16>>()
    }
    #[cfg(not(feature = "alloc"))]
    fn encode_lfn_utf16(_name: &str) -> () {
        ()
    }

    fn alloc_and_write_lfn_entries(
        &self,
        lfn_utf16: &LfnUtf16,
        short_name: &[u8; 11],
    ) -> io::Result<(DirRawStream<'a, T>, u64)> {
        // get short name checksum
        let lfn_chsum = lfn_checksum(short_name);
        // create LFN entries generator
        let lfn_iter = LfnEntriesGenerator::new(&lfn_utf16, lfn_chsum);
        // find space for new entries (multiple LFN entries and 1 SFN entry)
        let num_entries = lfn_iter.len() + 1;
        let mut stream = self.find_free_entries(num_entries)?;
        let start_pos = stream.seek(io::SeekFrom::Current(0))?;
        // write LFN entries before SFN entry
        for lfn_entry in lfn_iter {
            lfn_entry.serialize(&mut stream)?;
        }
        Ok((stream, start_pos))
    }

    fn write_entry(&self, name: &str, raw_entry: DirFileEntryData) -> io::Result<DirEntry<'a, T>> {
        trace!("write_entry {}", name);
        // check if name doesn't contain unsupported characters
        validate_long_name(name)?;
        // convert long name to UTF-16
        let lfn_utf16 = Self::encode_lfn_utf16(name);
        // write LFN entries
        let (mut stream, start_pos) = self.alloc_and_write_lfn_entries(&lfn_utf16, raw_entry.name())?;
        // write short name entry
        raw_entry.serialize(&mut stream)?;
        let end_pos = stream.seek(io::SeekFrom::Current(0))?;
        let abs_pos = stream.abs_pos().map(|p| p - DIR_ENTRY_SIZE);
        // return new logical entry descriptor
        let short_name = ShortName::new(raw_entry.name());
        Ok(DirEntry {
            data: raw_entry,
            short_name,
            lfn_utf16,
            fs: self.fs,
            entry_pos: abs_pos.unwrap(), // SAFE: abs_pos is absent only for empty file
            offset_range: (start_pos, end_pos),
        })
    }
}

// Note: derive cannot be used because of invalid bounds. See: https://github.com/rust-lang/rust/issues/26925
impl<'a, T: ReadWriteSeek> Clone for Dir<'a, T> {
    fn clone(&self) -> Self {
        Self { stream: self.stream.clone(), fs: self.fs }
    }
}

/// An iterator over the directory entries.
///
/// This struct is created by the `iter` method on `Dir`.
pub struct DirIter<'a, T: ReadWriteSeek + 'a> {
    stream: DirRawStream<'a, T>,
    fs: &'a FileSystem<T>,
    skip_volume: bool,
    err: bool,
}

impl<'a, T: ReadWriteSeek> DirIter<'a, T> {
    fn new(stream: DirRawStream<'a, T>, fs: &'a FileSystem<T>, skip_volume: bool) -> Self {
        DirIter { stream, fs, skip_volume, err: false }
    }

    fn should_ship_entry(&self, raw_entry: &DirEntryData) -> bool {
        if raw_entry.is_deleted() {
            return true;
        }
        match raw_entry {
            &DirEntryData::File(ref sfn_entry) => self.skip_volume && sfn_entry.is_volume(),
            _ => false,
        }
    }

    fn read_dir_entry(&mut self) -> io::Result<Option<DirEntry<'a, T>>> {
        trace!("read_dir_entry");
        let mut lfn_buf = LongNameBuilder::new();
        let mut offset = self.stream.seek(SeekFrom::Current(0))?;
        let mut begin_offset = offset;
        loop {
            let raw_entry = DirEntryData::deserialize(&mut self.stream)?;
            offset += DIR_ENTRY_SIZE;
            // Check if this is end of dir
            if raw_entry.is_end() {
                return Ok(None);
            }
            // Check if this is deleted or volume ID entry
            if self.should_ship_entry(&raw_entry) {
                trace!("skip entry");
                lfn_buf.clear();
                begin_offset = offset;
                continue;
            }
            match raw_entry {
                DirEntryData::File(data) => {
                    // Get entry position on volume
                    let abs_pos = self.stream.abs_pos().map(|p| p - DIR_ENTRY_SIZE);
                    // Check if LFN checksum is valid
                    lfn_buf.validate_chksum(data.name());
                    // Return directory entry
                    let short_name = ShortName::new(data.name());
                    trace!("file entry {:?}", data.name());
                    return Ok(Some(DirEntry {
                        data,
                        short_name,
                        lfn_utf16: lfn_buf.into_vec(),
                        fs: self.fs,
                        entry_pos: abs_pos.unwrap(), // SAFE: abs_pos is empty only for empty file
                        offset_range: (begin_offset, offset),
                    }));
                },
                DirEntryData::Lfn(data) => {
                    // Append to LFN buffer
                    trace!("lfn entry");
                    lfn_buf.process(&data);
                },
            }
        }
    }
}

// Note: derive cannot be used because of invalid bounds. See: https://github.com/rust-lang/rust/issues/26925
impl<'a, T: ReadWriteSeek> Clone for DirIter<'a, T> {
    fn clone(&self) -> Self {
        Self { stream: self.stream.clone(), fs: self.fs, err: self.err, skip_volume: self.skip_volume }
    }
}

impl<'a, T: ReadWriteSeek> Iterator for DirIter<'a, T> {
    type Item = io::Result<DirEntry<'a, T>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.err {
            return None;
        }
        let r = self.read_dir_entry();
        match r {
            Ok(Some(e)) => Some(Ok(e)),
            Ok(None) => None,
            Err(err) => {
                self.err = true;
                Some(Err(err))
            },
        }
    }
}

fn validate_long_name(name: &str) -> io::Result<()> {
    // check if length is valid
    if name.is_empty() {
        return Err(io::Error::new(ErrorKind::Other, "File name is empty"));
    }
    if name.len() > 255 {
        return Err(io::Error::new(ErrorKind::Other, "File name too long"));
    }
    // check if there are only valid characters
    for c in name.chars() {
        match c {
            'a'...'z' | 'A'...'Z' | '0'...'9' => {},
            '\u{80}'...'\u{FFFF}' => {},
            '$' | '%' | '\'' | '-' | '_' | '@' | '~' | '`' | '!' | '(' | ')' | '{' | '}' | '.' | ' ' | '+' | ','
            | ';' | '=' | '[' | ']' | '^' | '#' | '&' => {},
            _ => return Err(io::Error::new(ErrorKind::Other, "File name contains unsupported characters")),
        }
    }
    Ok(())
}

fn lfn_checksum(short_name: &[u8; 11]) -> u8 {
    let mut chksum = num::Wrapping(0u8);
    for b in short_name {
        chksum = (chksum << 7) + (chksum >> 1) + num::Wrapping(*b);
    }
    chksum.0
}

#[cfg(feature = "alloc")]
struct LongNameBuilder {
    buf: Vec<u16>,
    chksum: u8,
    index: u8,
}

#[cfg(feature = "alloc")]
impl LongNameBuilder {
    fn new() -> Self {
        LongNameBuilder { buf: Vec::<u16>::new(), chksum: 0, index: 0 }
    }

    fn clear(&mut self) {
        self.buf.clear();
        self.index = 0;
    }

    fn into_vec(mut self) -> Vec<u16> {
        // Check if last processed entry had index 1
        if self.index == 1 {
            self.truncate();
        } else if !self.is_empty() {
            warn!("unfinished LFN sequence {}", self.index);
            self.clear();
        }
        self.buf
    }

    fn truncate(&mut self) {
        // Truncate 0 and 0xFFFF characters from LFN buffer
        let mut lfn_len = self.buf.len();
        while lfn_len > 0 {
            match self.buf[lfn_len - 1] {
                0xFFFF | 0 => lfn_len -= 1,
                _ => break,
            }
        }
        self.buf.truncate(lfn_len);
    }

    fn is_empty(&self) -> bool {
        // Check if any LFN entry has been processed
        // Note: index 0 is not a valid index in LFN and can be seen only after struct initialization
        self.index == 0
    }

    fn process(&mut self, data: &DirLfnEntryData) {
        let is_last = (data.order() & LFN_ENTRY_LAST_FLAG) != 0;
        let index = data.order() & 0x1F;
        if index == 0 {
            // Corrupted entry
            warn!("currupted lfn entry! {:x}", data.order());
            self.clear();
            return;
        }
        if is_last {
            // last entry is actually first entry in stream
            self.index = index;
            self.chksum = data.checksum();
            self.buf.resize(index as usize * LFN_PART_LEN, 0);
        } else if self.index == 0 || index != self.index - 1 || data.checksum() != self.chksum {
            // Corrupted entry
            warn!("currupted lfn entry! {:x} {:x} {:x} {:x}", data.order(), self.index, data.checksum(), self.chksum);
            self.clear();
            return;
        } else {
            // Decrement LFN index only for non-last entries
            self.index -= 1;
        }
        let pos = LFN_PART_LEN * (index - 1) as usize;
        // copy name parts into LFN buffer
        data.copy_name_to_slice(&mut self.buf[pos..pos + 13]);
    }

    fn validate_chksum(&mut self, short_name: &[u8; 11]) {
        if self.is_empty() {
            // Nothing to validate - no LFN entries has been processed
            return;
        }
        let chksum = lfn_checksum(short_name);
        if chksum != self.chksum {
            warn!("checksum mismatch {:x} {:x} {:?}", chksum, self.chksum, short_name);
            self.clear();
        }
    }
}

// Dummy implementation for non-alloc build
#[cfg(not(feature = "alloc"))]
struct LongNameBuilder {}
#[cfg(not(feature = "alloc"))]
impl LongNameBuilder {
    fn new() -> Self {
        LongNameBuilder {}
    }
    fn clear(&mut self) {}
    fn into_vec(self) {}
    fn truncate(&mut self) {}
    fn process(&mut self, _data: &DirLfnEntryData) {}
    fn validate_chksum(&mut self, _short_name: &[u8; 11]) {}
}

#[cfg(feature = "alloc")]
struct LfnEntriesGenerator<'a> {
    name_parts_iter: iter::Rev<slice::Chunks<'a, u16>>,
    checksum: u8,
    index: usize,
    num: usize,
    ended: bool,
}

#[cfg(feature = "alloc")]
impl<'a> LfnEntriesGenerator<'a> {
    fn new(name_utf16: &'a [u16], checksum: u8) -> Self {
        let num_entries = (name_utf16.len() + LFN_PART_LEN - 1) / LFN_PART_LEN;
        // create generator using reverse iterator over chunks - first chunk can be shorter
        LfnEntriesGenerator {
            checksum,
            name_parts_iter: name_utf16.chunks(LFN_PART_LEN).rev(),
            index: 0,
            num: num_entries,
            ended: false,
        }
    }
}

#[cfg(feature = "alloc")]
impl<'a> Iterator for LfnEntriesGenerator<'a> {
    type Item = DirLfnEntryData;

    fn next(&mut self) -> Option<Self::Item> {
        if self.ended {
            return None;
        }

        // get next part from reverse iterator
        match self.name_parts_iter.next() {
            Some(ref name_part) => {
                let lfn_index = self.num - self.index;
                let mut order = lfn_index as u8;
                if self.index == 0 {
                    // this is last name part (written as first)
                    order |= LFN_ENTRY_LAST_FLAG;
                }
                debug_assert!(order > 0);
                // name is padded with ' '
                let mut lfn_part = [0xFFFFu16; LFN_PART_LEN];
                lfn_part[..name_part.len()].copy_from_slice(&name_part);
                if name_part.len() < LFN_PART_LEN {
                    // name is only zero-terminated if its length is not multiplicity of LFN_PART_LEN
                    lfn_part[name_part.len()] = 0;
                }
                // create and return new LFN entry
                let mut lfn_entry = DirLfnEntryData::new(order, self.checksum);
                lfn_entry.copy_name_from_slice(&lfn_part);
                self.index += 1;
                Some(lfn_entry)
            },
            None => {
                // end of name
                self.ended = true;
                None
            },
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.name_parts_iter.size_hint()
    }
}

// name_parts_iter is ExactSizeIterator so size_hint returns one limit
#[cfg(feature = "alloc")]
impl<'a> ExactSizeIterator for LfnEntriesGenerator<'a> {}

// Dummy implementation for non-alloc build
#[cfg(not(feature = "alloc"))]
struct LfnEntriesGenerator {}
#[cfg(not(feature = "alloc"))]
impl LfnEntriesGenerator {
    fn new(_name_utf16: &(), _checksum: u8) -> Self {
        LfnEntriesGenerator {}
    }
}
#[cfg(not(feature = "alloc"))]
impl Iterator for LfnEntriesGenerator {
    type Item = DirLfnEntryData;

    fn next(&mut self) -> Option<Self::Item> {
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(0))
    }
}
#[cfg(not(feature = "alloc"))]
impl ExactSizeIterator for LfnEntriesGenerator {}

#[derive(Default, Debug, Clone)]
struct ShortNameGenerator {
    chksum: u16,
    long_prefix_bitmap: u16,
    prefix_chksum_bitmap: u16,
    name_fits: bool,
    lossy_conv: bool,
    exact_match: bool,
    basename_len: u8,
    short_name: [u8; 11],
}

impl ShortNameGenerator {
    fn new(name: &str) -> Self {
        // padded by ' '
        let mut short_name = [SFN_PADDING; 11];
        // find extension after last dot
        let (basename_len, name_fits, lossy_conv) = match name.rfind('.') {
            Some(index) => {
                // extension found - copy parts before and after dot
                let (basename_len, basename_fits, basename_lossy) =
                    Self::copy_short_name_part(&mut short_name[0..8], &name[..index]);
                let (_, ext_fits, ext_lossy) = Self::copy_short_name_part(&mut short_name[8..11], &name[index + 1..]);
                (basename_len, basename_fits && ext_fits, basename_lossy || ext_lossy)
            },
            None => {
                // no extension - copy name and leave extension empty
                let (basename_len, basename_fits, basename_lossy) =
                    Self::copy_short_name_part(&mut short_name[0..8], &name);
                (basename_len, basename_fits, basename_lossy)
            },
        };
        let chksum = Self::checksum(name);
        Self { short_name, chksum, name_fits, lossy_conv, basename_len: basename_len as u8, ..Default::default() }
    }

    fn generate_dot() -> [u8; 11] {
        let mut short_name = [SFN_PADDING; 11];
        short_name[0] = 0x2e;
        short_name
    }

    fn generate_dotdot() -> [u8; 11] {
        let mut short_name = [SFN_PADDING; 11];
        short_name[0] = 0x2e;
        short_name[1] = 0x2e;
        short_name
    }

    fn copy_short_name_part(dst: &mut [u8], src: &str) -> (usize, bool, bool) {
        let mut dst_pos = 0;
        let mut lossy_conv = false;
        for c in src.chars() {
            if dst_pos == dst.len() {
                // result buffer is full
                return (dst_pos, false, lossy_conv);
            }
            // Make sure character is allowed in 8.3 name
            let fixed_c = match c {
                // strip spaces and dots
                ' ' | '.' => {
                    lossy_conv = true;
                    continue;
                },
                // copy allowed characters
                'A'...'Z' | 'a'...'z' | '0'...'9' => c,
                '!' | '#' | '$' | '%' | '&' | '\'' | '(' | ')' | '-' | '@' | '^' | '_' | '`' | '{' | '}' | '~' => c,
                // replace disallowed characters by underscore
                _ => '_',
            };
            // Update 'lossy conversion' flag
            lossy_conv = lossy_conv || (fixed_c != c);
            // short name is always uppercase
            let upper = fixed_c.to_ascii_uppercase();
            dst[dst_pos] = upper as u8; // SAFE: upper is in range 0x20-0x7F
            dst_pos += 1;
        }
        (dst_pos, true, lossy_conv)
    }

    fn add_existing(&mut self, short_name: &[u8; 11]) {
        // check for exact match collision
        if short_name == &self.short_name {
            self.exact_match = true;
        }
        // check for long prefix form collision (TEXTFI~1.TXT)
        let prefix_len = cmp::min(self.basename_len, 6) as usize;
        let num_suffix =
            if short_name[prefix_len] == b'~' { (short_name[prefix_len + 1] as char).to_digit(10) } else { None };
        let ext_matches = short_name[8..] == self.short_name[8..];
        if short_name[..prefix_len] == self.short_name[..prefix_len] && num_suffix.is_some() && ext_matches {
            let num = num_suffix.unwrap(); // SAFE
            self.long_prefix_bitmap |= 1 << num;
        }

        // check for short prefix + checksum form collision (TE021F~1.TXT)
        let prefix_len = cmp::min(self.basename_len, 2) as usize;
        let num_suffix = if short_name[prefix_len + 4] == b'~' {
            (short_name[prefix_len + 4 + 1] as char).to_digit(10)
        } else {
            None
        };
        if short_name[..prefix_len] == self.short_name[..prefix_len] && num_suffix.is_some() && ext_matches {
            let chksum_res =
                str::from_utf8(&short_name[prefix_len..prefix_len + 4]).map(|s| u16::from_str_radix(s, 16));
            if chksum_res == Ok(Ok(self.chksum)) {
                let num = num_suffix.unwrap(); // SAFE
                self.prefix_chksum_bitmap |= 1 << num;
            }
        }
    }

    fn checksum(name: &str) -> u16 {
        // BSD checksum algorithm
        let mut chksum = num::Wrapping(0u16);
        for c in name.chars() {
            chksum = (chksum >> 1) + (chksum << 15) + num::Wrapping(c as u16);
        }
        chksum.0
    }

    fn generate(&self) -> io::Result<[u8; 11]> {
        if !self.lossy_conv && self.name_fits && !self.exact_match {
            // If there was no lossy conversion and name fits into
            // 8.3 convention and there is no collision return it as is
            return Ok(self.short_name);
        }
        // Try using long 6-characters prefix
        for i in 1..5 {
            if self.long_prefix_bitmap & (1 << i) == 0 {
                return Ok(self.build_prefixed_name(i, false));
            }
        }
        // Try prefix with checksum
        for i in 1..10 {
            if self.prefix_chksum_bitmap & (1 << i) == 0 {
                return Ok(self.build_prefixed_name(i, true));
            }
        }
        // Too many collisions - fail
        Err(io::Error::new(ErrorKind::AlreadyExists, "short name already exists"))
    }

    fn next_iteration(&mut self) {
        // Try different checksum in next iteration
        self.chksum = (num::Wrapping(self.chksum) + num::Wrapping(1)).0;
        // Zero bitmaps
        self.long_prefix_bitmap = 0;
        self.prefix_chksum_bitmap = 0;
    }

    fn build_prefixed_name(&self, num: u32, with_chksum: bool) -> [u8; 11] {
        let mut buf = [SFN_PADDING; 11];
        let prefix_len = if with_chksum {
            let prefix_len = cmp::min(self.basename_len as usize, 2);
            buf[..prefix_len].copy_from_slice(&self.short_name[..prefix_len]);
            buf[prefix_len..prefix_len + 4].copy_from_slice(&Self::u16_to_u8_array(self.chksum));
            prefix_len + 4
        } else {
            let prefix_len = cmp::min(self.basename_len as usize, 6);
            buf[..prefix_len].copy_from_slice(&self.short_name[..prefix_len]);
            prefix_len
        };
        buf[prefix_len] = b'~';
        buf[prefix_len + 1] = char::from_digit(num, 10).unwrap() as u8; // SAFE
        buf[8..].copy_from_slice(&self.short_name[8..]);
        buf
    }

    fn u16_to_u8_array(x: u16) -> [u8; 4] {
        let c1 = char::from_digit((x as u32 >> 12) & 0xF, 16).unwrap().to_ascii_uppercase() as u8;
        let c2 = char::from_digit((x as u32 >> 8) & 0xF, 16).unwrap().to_ascii_uppercase() as u8;
        let c3 = char::from_digit((x as u32 >> 4) & 0xF, 16).unwrap().to_ascii_uppercase() as u8;
        let c4 = char::from_digit((x as u32 >> 0) & 0xF, 16).unwrap().to_ascii_uppercase() as u8;
        [c1, c2, c3, c4]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_path() {
        assert_eq!(split_path("aaa/bbb/ccc"), ("aaa", Some("bbb/ccc")));
        assert_eq!(split_path("aaa/bbb"), ("aaa", Some("bbb")));
        assert_eq!(split_path("aaa"), ("aaa", None));
    }

    #[test]
    fn test_generate_short_name() {
        assert_eq!(&ShortNameGenerator::new("Foo").generate().unwrap(), b"FOO        ");
        assert_eq!(&ShortNameGenerator::new("Foo.b").generate().unwrap(), b"FOO     B  ");
        assert_eq!(&ShortNameGenerator::new("Foo.baR").generate().unwrap(), b"FOO     BAR");
        assert_eq!(&ShortNameGenerator::new("Foo+1.baR").generate().unwrap(), b"FOO_1~1 BAR");
        assert_eq!(&ShortNameGenerator::new("ver +1.2.text").generate().unwrap(), b"VER_12~1TEX");
        assert_eq!(&ShortNameGenerator::new(".bashrc.swp").generate().unwrap(), b"BASHRC~1SWP");
    }

    #[test]
    fn test_short_name_checksum_overflow() {
        ShortNameGenerator::checksum("\u{FF5A}\u{FF5A}\u{FF5A}\u{FF5A}");
    }

    #[test]
    fn test_lfn_checksum_overflow() {
        lfn_checksum(&[0xFFu8, 0xFFu8, 0xFFu8, 0xFFu8, 0xFFu8, 0xFFu8, 0xFFu8, 0xFFu8, 0xFFu8, 0xFFu8, 0xFFu8]);
    }

    #[test]
    fn test_generate_short_name_collisions_long() {
        let mut buf: [u8; 11];
        let mut gen = ShortNameGenerator::new("TextFile.Mine.txt");
        buf = gen.generate().unwrap();
        assert_eq!(&buf, b"TEXTFI~1TXT");
        gen.add_existing(&buf);
        buf = gen.generate().unwrap();
        assert_eq!(&buf, b"TEXTFI~2TXT");
        gen.add_existing(&buf);
        buf = gen.generate().unwrap();
        assert_eq!(&buf, b"TEXTFI~3TXT");
        gen.add_existing(&buf);
        buf = gen.generate().unwrap();
        assert_eq!(&buf, b"TEXTFI~4TXT");
        gen.add_existing(&buf);
        buf = gen.generate().unwrap();
        assert_eq!(&buf, b"TE527D~1TXT");
        gen.add_existing(&buf);
        buf = gen.generate().unwrap();
        assert_eq!(&buf, b"TE527D~2TXT");
        for i in 3..10 {
            gen.add_existing(&buf);
            buf = gen.generate().unwrap();
            assert_eq!(&buf, format!("TE527D~{}TXT", i).as_bytes());
        }
        gen.add_existing(&buf);
        assert!(gen.generate().is_err());
        gen.next_iteration();
        for _i in 0..4 {
            buf = gen.generate().unwrap();
            gen.add_existing(&buf);
        }
        buf = gen.generate().unwrap();
        assert_eq!(&buf, b"TE527E~1TXT");
    }

    #[test]
    fn test_generate_short_name_collisions_short() {
        let mut buf: [u8; 11];
        let mut gen = ShortNameGenerator::new("x.txt");
        buf = gen.generate().unwrap();
        assert_eq!(&buf, b"X       TXT");
        gen.add_existing(&buf);
        buf = gen.generate().unwrap();
        assert_eq!(&buf, b"X~1     TXT");
        gen.add_existing(&buf);
        buf = gen.generate().unwrap();
        assert_eq!(&buf, b"X~2     TXT");
        gen.add_existing(&buf);
        buf = gen.generate().unwrap();
        assert_eq!(&buf, b"X~3     TXT");
        gen.add_existing(&buf);
        buf = gen.generate().unwrap();
        assert_eq!(&buf, b"X~4     TXT");
        gen.add_existing(&buf);
        buf = gen.generate().unwrap();
        assert_eq!(&buf, b"X40DA~1 TXT");
        gen.add_existing(&buf);
        buf = gen.generate().unwrap();
        assert_eq!(&buf, b"X40DA~2 TXT");
    }
}
