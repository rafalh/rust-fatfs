#[cfg(feature = "alloc")]
use core::{slice, iter};
use core::{str, char, cmp};

use io::prelude::*;
use io;
use io::{ErrorKind, SeekFrom};

use fs::{FileSystemRef, DiskSlice};
use file::File;
use dir_entry::{DirEntry, DirEntryData, DirFileEntryData, DirLfnEntryData, FileAttributes, ShortName, DIR_ENTRY_SIZE};

#[cfg(feature = "alloc")]
use dir_entry::{LFN_PART_LEN, LFN_ENTRY_LAST_FLAG};

#[cfg(all(not(feature = "std"), feature = "alloc"))]
use alloc::Vec;

#[derive(Clone)]
pub(crate) enum DirRawStream<'a, 'b: 'a> {
    File(File<'a, 'b>),
    Root(DiskSlice<'a, 'b>),
}

impl <'a, 'b> DirRawStream<'a, 'b> {
    pub(crate) fn abs_pos(&self) -> Option<u64> {
        match self {
            &DirRawStream::File(ref file) => file.abs_pos(),
            &DirRawStream::Root(ref slice) => Some(slice.abs_pos()),
        }
    }

    pub(crate) fn first_cluster(&self) -> Option<u32> {
        match self {
            &DirRawStream::File(ref file) => file.first_cluster(),
            &DirRawStream::Root(_) => None,
        }
    }
}

impl <'a, 'b> Read for DirRawStream<'a, 'b> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            &mut DirRawStream::File(ref mut file) => file.read(buf),
            &mut DirRawStream::Root(ref mut raw) => raw.read(buf),
        }
    }
}

impl <'a, 'b> Write for DirRawStream<'a, 'b> {
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

impl <'a, 'b> Seek for DirRawStream<'a, 'b> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match self {
            &mut DirRawStream::File(ref mut file) => file.seek(pos),
            &mut DirRawStream::Root(ref mut raw) => raw.seek(pos),
        }
    }
}

fn split_path<'c>(path: &'c str) -> (&'c str, Option<&'c str>) {
    // remove trailing slash and split into 2 components - top-most parent and rest
    let mut path_split = path.trim_matches('/').splitn(2, "/");
    let comp = path_split.next().unwrap(); // SAFE: splitn always returns at least one element
    let rest_opt = path_split.next();
    (comp, rest_opt)
}

/// FAT directory
#[derive(Clone)]
pub struct Dir<'a, 'b: 'a> {
    stream: DirRawStream<'a, 'b>,
    fs: FileSystemRef<'a, 'b>,
}

impl <'a, 'b> Dir<'a, 'b> {

    pub(crate) fn new(stream: DirRawStream<'a, 'b>, fs: FileSystemRef<'a, 'b>) -> Dir<'a, 'b> {
        Dir { stream, fs }
    }

    /// Creates directory entries iterator
    pub fn iter(&self) -> DirIter<'a, 'b> {
        DirIter {
            stream: self.stream.clone(),
            fs: self.fs.clone(),
            err: false,
        }
    }

    fn find_entry(&mut self, name: &str, mut short_name_gen: Option<&mut ShortNameGenerator>) -> io::Result<DirEntry<'a, 'b>> {
        for r in self.iter() {
            let e = r?;
            // compare name ignoring case
            if e.file_name().eq_ignore_ascii_case(name) {
                return Ok(e);
            }
            if let Some(ref mut gen) = short_name_gen {
                gen.add_existing(e.raw_short_name());
            }
        }
        Err(io::Error::new(ErrorKind::NotFound, "file not found"))
    }

    /// Opens existing directory
    pub fn open_dir(&mut self, path: &str) -> io::Result<Dir<'a, 'b>> {
        let (name, rest_opt) = split_path(path);
        let e = self.find_entry(name, None)?;
        match rest_opt {
            Some(rest) => e.to_dir().open_dir(rest),
            None => Ok(e.to_dir())
        }
    }

    /// Opens existing file.
    pub fn open_file(&mut self, path: &str) -> io::Result<File<'a, 'b>> {
        let (name, rest_opt) = split_path(path);
        let e = self.find_entry(name, None)?;
        match rest_opt {
            Some(rest) => e.to_dir().open_file(rest),
            None => Ok(e.to_file())
        }
    }

    /// Creates new file or opens existing without truncating.
    pub fn create_file(&mut self, path: &str) -> io::Result<File<'a, 'b>> {
        let (name, rest_opt) = split_path(path);
        match rest_opt {
            // path contains more than 1 component
            Some(rest) => self.find_entry(name, None)?.to_dir().create_file(rest),
            None => {
                // this is final filename in the path
                let mut short_name_gen = ShortNameGenerator::new(name);
                let r = self.find_entry(name, Some(&mut short_name_gen));
                match r {
                    Err(ref err) if err.kind() == ErrorKind::NotFound => {
                        let short_name = short_name_gen.generate()?;
                        Ok(self.create_entry(name, short_name, FileAttributes::from_bits_truncate(0), None)?.to_file())
                    },
                    Err(err) => Err(err),
                    Ok(e) => Ok(e.to_file()),
                }
            }
        }
    }

    /// Creates new directory or opens existing.
    pub fn create_dir(&mut self, path: &str) -> io::Result<Dir<'a, 'b>> {
        let (name, rest_opt) = split_path(path);
        match rest_opt {
            // path contains more than 1 component
            Some(rest) => self.find_entry(name, None)?.to_dir().create_dir(rest),
            None => {
                // this is final filename in the path
                let mut short_name_gen = ShortNameGenerator::new(name);
                let r = self.find_entry(name, Some(&mut short_name_gen));
                match r {
                    Err(ref err) if err.kind() == ErrorKind::NotFound => {
                        // alloc cluster for directory data
                        let cluster = self.fs.alloc_cluster(None)?;
                        // create entry in parent directory
                        let short_name = short_name_gen.generate()?;
                        let entry = self.create_entry(name, short_name, FileAttributes::DIRECTORY, Some(cluster))?;
                        let mut dir = entry.to_dir();
                        // create special entries "." and ".."
                        let dot_sfn = ShortNameGenerator::new(".").generate().unwrap();
                        dir.create_entry(".", dot_sfn, FileAttributes::DIRECTORY, entry.first_cluster())?;
                        let dotdot_sfn = ShortNameGenerator::new("..").generate().unwrap();
                        dir.create_entry("..", dotdot_sfn, FileAttributes::DIRECTORY, self.stream.first_cluster())?;
                        Ok(dir)
                    },
                    Err(err) => Err(err),
                    Ok(e) => Ok(e.to_dir()),
                }
            }
        }
    }

    fn is_empty(&mut self) -> io::Result<bool> {
        // check if directory contains no files
        for r in self.iter() {
            let e = r?;
            let name = e.file_name();
            // ignore special entries "." and ".."
            if name != "." && name != ".." {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Removes existing file or directory.
    ///
    /// Make sure there is no reference to this file (no File instance) or filesystem corruption
    /// can happen.
    pub fn remove(&mut self, path: &str) -> io::Result<()> {
        let (name, rest_opt) = split_path(path);
        let e = self.find_entry(name, None)?;
        match rest_opt {
            Some(rest) => e.to_dir().remove(rest),
            None => {
                trace!("removing {}", path);
                // in case of directory check if it is empty
                if e.is_dir() && !e.to_dir().is_empty()? {
                    return Err(io::Error::new(ErrorKind::NotFound, "removing non-empty directory is denied"));
                }
                // free directory data
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
                    data.set_free();
                    stream.seek(SeekFrom::Current(-(DIR_ENTRY_SIZE as i64)))?;
                    data.serialize(&mut stream)?;
                }
                Ok(())
            }
        }
    }

    fn find_free_entries(&mut self, num_entries: usize) -> io::Result<DirRawStream<'a, 'b>> {
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
            } else if raw_entry.is_free() {
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

    #[cfg(feature = "alloc")]
    fn create_lfn_entries(&mut self, name: &str, short_name: &[u8]) -> io::Result<(DirRawStream<'a, 'b>, u64)> {
        // get short name checksum
        let lfn_chsum = lfn_checksum(&short_name);
        // convert long name to UTF-16
        let lfn_utf16 = name.encode_utf16().collect::<Vec<u16>>();
        let lfn_iter = LfnEntriesGenerator::new(&lfn_utf16, lfn_chsum);
        // find space for new entries
        let num_entries = lfn_iter.len() + 1; // multiple lfn entries + one file entry
        let mut stream = self.find_free_entries(num_entries)?;
        let start_pos = stream.seek(io::SeekFrom::Current(0))?;
        // write LFN entries first
        for lfn_entry in lfn_iter {
            lfn_entry.serialize(&mut stream)?;
        }
        Ok((stream, start_pos))
    }
    #[cfg(not(feature = "alloc"))]
    fn create_lfn_entries(&mut self, _name: &str, _short_name: &[u8]) -> io::Result<(DirRawStream<'a, 'b>, u64)> {
        let mut stream = self.find_free_entries(1)?;
        let start_pos = stream.seek(io::SeekFrom::Current(0))?;
        Ok((stream, start_pos))
    }

    fn create_entry(&mut self, name: &str, short_name: [u8; 11], attrs: FileAttributes, first_cluster: Option<u32>) -> io::Result<DirEntry<'a, 'b>> {
        trace!("create_entry {}", name);
        // check if name doesn't contain unsupported characters
        validate_long_name(name)?;
        // generate long entries
        let (mut stream, start_pos) = self.create_lfn_entries(&name, &short_name)?;
        // create and write short name entry
        let mut raw_entry = DirFileEntryData::new(short_name, attrs);
        raw_entry.set_first_cluster(first_cluster, self.fs.fat_type());
        raw_entry.reset_created();
        raw_entry.reset_accessed();
        raw_entry.reset_modified();
        raw_entry.serialize(&mut stream)?;
        let end_pos = stream.seek(io::SeekFrom::Current(0))?;
        let abs_pos = stream.abs_pos().map(|p| p - DIR_ENTRY_SIZE);
        // return new logical entry descriptor
        let short_name = ShortName::new(raw_entry.name());
        return Ok(DirEntry {
            data: raw_entry,
            short_name,
            #[cfg(feature = "alloc")]
            lfn: Vec::new(),
            fs: self.fs,
            entry_pos: abs_pos.unwrap(), // SAFE: abs_pos is absent only for empty file
            offset_range: (start_pos, end_pos),
        });
    }
}

/// Directory entries iterator.
#[derive(Clone)]
pub struct DirIter<'a, 'b: 'a> {
    stream: DirRawStream<'a, 'b>,
    fs: FileSystemRef<'a, 'b>,
    err: bool,
}

impl <'a, 'b> DirIter<'a, 'b> {
    fn read_dir_entry(&mut self) -> io::Result<Option<DirEntry<'a, 'b>>> {
        #[cfg(feature = "alloc")]
        let mut lfn_buf = LongNameBuilder::new();
        let mut offset = self.stream.seek(SeekFrom::Current(0))?;
        let mut begin_offset = offset;
        loop {
            let raw_entry = DirEntryData::deserialize(&mut self.stream)?;
            offset += DIR_ENTRY_SIZE;
            match raw_entry {
                DirEntryData::File(data) => {
                    // Check if this is end of dif
                    if data.is_end() {
                        return Ok(None);
                    }
                    // Check if this is deleted or volume ID entry
                    if data.is_free() || data.is_volume() {
                        #[cfg(feature = "alloc")]
                        lfn_buf.clear();
                        begin_offset = offset;
                        continue;
                    }
                    // Get entry position on volume
                    let abs_pos = self.stream.abs_pos().map(|p| p - DIR_ENTRY_SIZE);
                    // Check if LFN checksum is valid
                    #[cfg(feature = "alloc")]
                    lfn_buf.validate_chksum(data.name());
                    // Return directory entry
                    let short_name = ShortName::new(data.name());
                    return Ok(Some(DirEntry {
                        data,
                        short_name,
                        #[cfg(feature = "alloc")]
                        lfn: lfn_buf.to_vec(),
                        fs: self.fs,
                        entry_pos: abs_pos.unwrap(), // SAFE: abs_pos is empty only for empty file
                        offset_range: (begin_offset, offset),
                    }));
                },
                DirEntryData::Lfn(data) => {
                    // Check if this is deleted entry
                    if data.is_free() {
                        #[cfg(feature = "alloc")]
                        lfn_buf.clear();
                        begin_offset = offset;
                        continue;
                    }
                    // Append to LFN buffer
                    #[cfg(feature = "alloc")]
                    lfn_buf.process(&data);
                }
            }
        }
    }
}

impl <'a, 'b> Iterator for DirIter<'a, 'b> {
    type Item = io::Result<DirEntry<'a, 'b>>;

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
    if name.len() == 0 {
        return Err(io::Error::new(ErrorKind::InvalidInput, "filename cannot be empty"));
    }
    if name.len() > 255 {
        return Err(io::Error::new(ErrorKind::InvalidInput, "filename is too long"));
    }
    for c in name.chars() {
        match c {
            'a'...'z' | 'A'...'Z' | '0'...'9' | '\u{80}'...'\u{FFFF}' |
            '$' | '%' | '\'' | '-' | '_' | '@' | '~' | '`' | '!' | '(' | ')' | '{' | '}' |
            '.' | ' ' | '+' | ',' | ';' | '=' | '[' | ']' => {},
            _ => return Err(io::Error::new(ErrorKind::InvalidInput, "invalid character in filename")),
        }
    }
    Ok(())
}

#[cfg(feature = "alloc")]
fn lfn_checksum(short_name: &[u8]) -> u8 {
    let mut chksum = 0u8;
    for i in 0..11 {
        chksum = (((chksum & 1) << 7) as u16 + (chksum >> 1) as u16 + short_name[i] as u16) as u8;
    }
    chksum
}

#[cfg(feature = "alloc")]
struct LongNameBuilder {
    buf: Vec<u16>,
    chksum: u8,
    index: u8,
}

#[cfg(feature = "alloc")]
impl LongNameBuilder {
    fn new() -> LongNameBuilder {
        LongNameBuilder {
            buf: Vec::<u16>::new(),
            chksum: 0,
            index: 0,
        }
    }

    fn clear(&mut self) {
        self.buf.clear();
        self.index = 0;
    }

    fn to_vec(mut self) -> Vec<u16> {
        if self.index == 1 {
            self.truncate();
            self.buf
        } else {
            warn!("unfinished LFN sequence {}", self.index);
            Vec::<u16>::new()
        }
    }

    fn truncate(&mut self) {
        // Truncate 0 and 0xFFFF characters from LFN buffer
        let mut lfn_len = self.buf.len();
        while lfn_len > 0 {
            match self.buf[lfn_len-1] {
                0xFFFF | 0 => lfn_len -= 1,
                _ => break,
            }
        }
        self.buf.truncate(lfn_len);
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
        data.copy_name_to_slice(&mut self.buf[pos..pos+13]);
    }

    fn validate_chksum(&mut self, short_name: &[u8]) {
        let chksum = lfn_checksum(short_name);
        if chksum != self.chksum {
            warn!("checksum mismatch {:x} {:x} {:?}", chksum, self.chksum, short_name);
            self.clear();
        }
    }
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
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.name_parts_iter.size_hint()
    }
}

// name_parts_iter is ExactSizeIterator so size_hint returns one limit
#[cfg(feature = "alloc")]
impl<'a> ExactSizeIterator for LfnEntriesGenerator<'a> {}

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
        let mut short_name = [0x20u8; 11];
        // find extension after last dot
        let (basename_len, name_fits, lossy_conv) = match name.rfind('.') {
            Some(index) => {
                // extension found - copy parts before and after dot
                let (basename_len, basename_fits, basename_lossy) = Self::copy_short_name_part(&mut short_name[0..8], &name[..index]);
                let (_, ext_fits, ext_lossy) = Self::copy_short_name_part(&mut short_name[8..11], &name[index+1..]);
                (basename_len, basename_fits && ext_fits, basename_lossy || ext_lossy)
            },
            None => {
                // no extension - copy name and leave extension empty
                let (basename_len, basename_fits, basename_lossy) = Self::copy_short_name_part(&mut short_name[0..8], &name);
                (basename_len, basename_fits, basename_lossy)
            }
        };
        let chksum = Self::checksum(name);
        Self {
            short_name, chksum, name_fits, lossy_conv,
            basename_len: basename_len as u8,
            ..Default::default()
        }
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
                'A'...'Z' | 'a'...'z' | '0'...'9' |
                '!' | '#' | '$' | '%' | '&' | '\'' | '(' | ')' |
                '-' | '@' | '^' | '_' | '`' | '{' | '}' | '~' => c,
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
        let num_suffix = if short_name[prefix_len] as char == '~' { (short_name[prefix_len+1] as char).to_digit(10) } else { None };
        let ext_matches = short_name[8..] == self.short_name[8..];
        if short_name[..prefix_len] == self.short_name[..prefix_len] && num_suffix.is_some() && ext_matches {
            let num = num_suffix.unwrap(); // SAFE
            self.long_prefix_bitmap |= 1 << num;
        }

        // check for short prefix + checksum form collision (TE021F~1.TXT)
        let prefix_len = cmp::min(self.basename_len, 2) as usize;
        let num_suffix = if short_name[prefix_len+4] as char == '~' { (short_name[prefix_len+4+1] as char).to_digit(10) } else { None };
        if short_name[..prefix_len] == self.short_name[..prefix_len] && num_suffix.is_some() && ext_matches {
            let chksum_res = str::from_utf8(&short_name[prefix_len..prefix_len+4]).map(|s| u16::from_str_radix(s, 16));
            if chksum_res == Ok(Ok(self.chksum)) {
                let num = num_suffix.unwrap(); // SAFE
                self.prefix_chksum_bitmap |= 1 << num;
            }
        }
    }

    fn checksum(name: &str) -> u16 {
        // BSD checksum algorithm
        let mut chksum = 0u16;
        for c in name.chars() {
            chksum = (chksum >> 1) + ((chksum & 1) << 15) + (c as u16);
        }
        chksum
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

    fn build_prefixed_name(&self, num: u32, with_chksum: bool) -> [u8; 11] {
        let mut buf = [0x20u8; 11];
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
        buf[prefix_len] = '~' as u8;
        buf[prefix_len + 1] = char::from_digit(num, 10).unwrap() as u8; // SAFE
        buf[8..].copy_from_slice(&self.short_name[8..]);
        buf
    }

    fn u16_to_u8_array(x: u16) -> [u8;4] {
        let c1 = char::from_digit((x as u32 >> 12) & 0xF, 16).unwrap().to_ascii_uppercase() as u8;
        let c2 = char::from_digit((x as u32 >> 8) & 0xF, 16).unwrap().to_ascii_uppercase() as u8;
        let c3 = char::from_digit((x as u32 >> 4) & 0xF, 16).unwrap().to_ascii_uppercase() as u8;
        let c4 = char::from_digit((x as u32 >> 0) & 0xF, 16).unwrap().to_ascii_uppercase() as u8;
        return [c1, c2, c3, c4]
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
        assert_eq!(&ShortNameGenerator::new("Foo").generate().unwrap(), "FOO        ".as_bytes());
        assert_eq!(&ShortNameGenerator::new("Foo.b").generate().unwrap(), "FOO     B  ".as_bytes());
        assert_eq!(&ShortNameGenerator::new("Foo.baR").generate().unwrap(), "FOO     BAR".as_bytes());
        assert_eq!(&ShortNameGenerator::new("Foo+1.baR").generate().unwrap(), "FOO_1~1 BAR".as_bytes());
        assert_eq!(&ShortNameGenerator::new("ver +1.2.text").generate().unwrap(), "VER_12~1TEX".as_bytes());
        assert_eq!(&ShortNameGenerator::new(".bashrc.swp").generate().unwrap(), "BASHRC~1SWP".as_bytes());
    }

    #[test]
    fn test_generate_short_name_collisions_long() {
        let mut buf: [u8; 11];
        let mut gen = ShortNameGenerator::new("TextFile.Mine.txt");
        buf = gen.generate().unwrap();
        assert_eq!(&buf, "TEXTFI~1TXT".as_bytes());
        gen.add_existing(&buf);
        buf = gen.generate().unwrap();
        assert_eq!(&buf, "TEXTFI~2TXT".as_bytes());
        gen.add_existing(&buf);
        buf = gen.generate().unwrap();
        assert_eq!(&buf, "TEXTFI~3TXT".as_bytes());
        gen.add_existing(&buf);
        buf = gen.generate().unwrap();
        assert_eq!(&buf, "TEXTFI~4TXT".as_bytes());
        gen.add_existing(&buf);
        buf = gen.generate().unwrap();
        assert_eq!(&buf, "TE527D~1TXT".as_bytes());
        gen.add_existing(&buf);
        buf = gen.generate().unwrap();
        assert_eq!(&buf, "TE527D~2TXT".as_bytes());
    }

    #[test]
    fn test_generate_short_name_collisions_short() {
        let mut buf: [u8; 11];
        let mut gen = ShortNameGenerator::new("x.txt");
        buf = gen.generate().unwrap();
        assert_eq!(&buf, "X       TXT".as_bytes());
        gen.add_existing(&buf);
        buf = gen.generate().unwrap();
        assert_eq!(&buf, "X~1     TXT".as_bytes());
        gen.add_existing(&buf);
        buf = gen.generate().unwrap();
        assert_eq!(&buf, "X~2     TXT".as_bytes());
        gen.add_existing(&buf);
        buf = gen.generate().unwrap();
        assert_eq!(&buf, "X~3     TXT".as_bytes());
        gen.add_existing(&buf);
        buf = gen.generate().unwrap();
        assert_eq!(&buf, "X~4     TXT".as_bytes());
        gen.add_existing(&buf);
        buf = gen.generate().unwrap();
        assert_eq!(&buf, "X40DA~1 TXT".as_bytes());
        gen.add_existing(&buf);
        buf = gen.generate().unwrap();
        assert_eq!(&buf, "X40DA~2 TXT".as_bytes());
    }
}
