use core::{fmt, str};
use core::iter::FromIterator;
use core::char;
use io::prelude::*;
use io;
use io::Cursor;
use byteorder::{LittleEndian};
use byteorder_ext::{ReadBytesExt, WriteBytesExt};

#[cfg(feature = "chrono")]
use chrono::{TimeZone, Local, Datelike, Timelike};
#[cfg(feature = "chrono")]
use chrono;

#[cfg(all(not(feature = "std"), feature = "alloc"))]
use alloc::{Vec, String, string::ToString};

use fs::{FileSystem, FatType, ReadWriteSeek, decode_oem_char_lossy};
use file::File;
use dir::{Dir, DirRawStream};

bitflags! {
    /// A FAT file attributes.
    #[derive(Default)]
    pub struct FileAttributes: u8 {
        const READ_ONLY  = 0x01;
        const HIDDEN     = 0x02;
        const SYSTEM     = 0x04;
        const VOLUME_ID  = 0x08;
        const DIRECTORY  = 0x10;
        const ARCHIVE    = 0x20;
        const LFN        = Self::READ_ONLY.bits | Self::HIDDEN.bits
                         | Self::SYSTEM.bits | Self::VOLUME_ID.bits;
    }
}

// Size of single directory entry in bytes
pub(crate) const DIR_ENTRY_SIZE: u64 = 32;

// Directory entry flags available in first byte of the short name
pub(crate) const DIR_ENTRY_DELETED_FLAG: u8 = 0xE5;
pub(crate) const DIR_ENTRY_REALLY_E5_FLAG: u8 = 0x05;

// Length in characters of a LFN fragment packed in one directory entry
pub(crate) const LFN_PART_LEN: usize = 13;

// Bit used in order field to mark last LFN entry
pub(crate) const LFN_ENTRY_LAST_FLAG: u8 = 0x40;

/// Decoded file short name
#[derive(Clone, Debug, Default)]
pub(crate) struct ShortName {
    name: [u8; 12],
    len: u8,
}

impl ShortName {
    const PADDING: u8 = ' ' as u8;

    pub(crate) fn new(raw_name: &[u8; 11]) -> Self {
        // get name components length by looking for space character
        let name_len = raw_name[0..8].iter().rposition(|x| *x != Self::PADDING).map(|p| p + 1).unwrap_or(0);
        let ext_len = raw_name[8..11].iter().rposition(|x| *x != Self::PADDING).map(|p| p + 1).unwrap_or(0);
        let mut name = [Self::PADDING; 12];
        name[..name_len].copy_from_slice(&raw_name[..name_len]);
        let total_len = if ext_len > 0 {
            name[name_len] = '.' as u8;
            name[name_len+1..name_len+1+ext_len].copy_from_slice(&raw_name[8..8+ext_len]);
            // Return total name length
            name_len+1+ext_len
        } else {
            // No extension - return length of name part
            name_len
        };
        // FAT encodes character 0xE5 as 0x05 because 0xE5 marks deleted files
        if name[0] == DIR_ENTRY_REALLY_E5_FLAG {
            name[0] = 0xE5;
        }
        // Short names in FAT filesystem are encoded in OEM code-page
        ShortName {
            name,
            len: total_len as u8,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.name[..self.len as usize]
    }

    #[cfg(feature = "alloc")]
    fn to_string(&self) -> String {
        // Strip non-ascii characters from short name
        let char_iter = self.as_bytes().iter().cloned().map(decode_oem_char_lossy);
        // Build string from character iterator
        String::from_iter(char_iter)
    }

    fn eq_ignore_ascii_case(&self, name: &str) -> bool {
        // Strip non-ascii characters from short name
        let char_iter = self.as_bytes().iter().cloned().map(decode_oem_char_lossy).map(|c| c.to_ascii_uppercase());
        // Build string from character iterator
        char_iter.eq(name.chars().map(|c| c.to_ascii_uppercase()))
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub(crate) struct DirFileEntryData {
    name: [u8; 11],
    attrs: FileAttributes,
    reserved_0: u8,
    create_time_0: u8,
    create_time_1: u16,
    create_date: u16,
    access_date: u16,
    first_cluster_hi: u16,
    modify_time: u16,
    modify_date: u16,
    first_cluster_lo: u16,
    size: u32,
}

impl DirFileEntryData {
    pub(crate) fn new(name: [u8; 11], attrs: FileAttributes) -> Self {
        DirFileEntryData {
            name, attrs,
            ..Default::default()
        }
    }

    pub(crate) fn renamed(&self, new_name: [u8; 11]) -> Self {
        let mut sfn_entry = self.clone();
        sfn_entry.name = new_name;
        sfn_entry
    }

    pub(crate) fn name(&self) -> &[u8; 11] {
        &self.name
    }

    #[cfg(feature = "alloc")]
    fn lowercase_name(&self) -> String {
        let mut name_copy: [u8; 11] = self.name;
        if self.lowercase_basename() {
            for c in &mut name_copy[..8] {
                *c = (*c as char).to_ascii_lowercase() as u8;
            }
        }
        if self.lowercase_ext() {
            for c in &mut name_copy[8..] {
                *c = (*c as char).to_ascii_lowercase() as u8;
            }
        }
        ShortName::new(&name_copy).to_string()
    }

    pub(crate) fn first_cluster(&self, fat_type: FatType) -> Option<u32> {
        let first_cluster_hi = if fat_type == FatType::Fat32 { self.first_cluster_hi } else { 0 };
        let n = ((first_cluster_hi as u32) << 16) | self.first_cluster_lo as u32;
        if n == 0 { None } else { Some(n) }
    }

    pub(crate) fn set_first_cluster(&mut self, cluster: Option<u32>, fat_type: FatType) {
        let n = cluster.unwrap_or(0);
        if fat_type == FatType::Fat32 {
            self.first_cluster_hi = (n >> 16) as u16;
        }
        self.first_cluster_lo = (n & 0xFFFF) as u16;
    }

    pub(crate) fn size(&self) -> Option<u32> {
        if self.is_file() {
            Some(self.size)
        } else {
            None
        }
    }

    fn set_size(&mut self, size: u32) {
        self.size = size;
    }

    fn is_dir(&self) -> bool {
        self.attrs.contains(FileAttributes::DIRECTORY)
    }

    fn is_file(&self) -> bool {
        !self.is_dir()
    }

    fn lowercase_basename(&self) -> bool {
        self.reserved_0 & (1 << 3) != 0
    }

    fn lowercase_ext(&self) -> bool {
        self.reserved_0 & (1 << 4) != 0
    }

    fn created(&self) -> DateTime {
        DateTime::decode(self.create_date, self.create_time_1, self.create_time_0)
    }

    fn accessed(&self) -> Date {
        Date::decode(self.access_date)
    }

    fn modified(&self) -> DateTime {
        DateTime::decode(self.modify_date, self.modify_time, 0)
    }

    fn set_created(&mut self, date_time: DateTime) {
        self.create_date = date_time.date.encode();
        let encoded_time = date_time.time.encode();
        self.create_time_1 = encoded_time.0;
        self.create_time_0 = encoded_time.1;
    }

    fn set_accessed(&mut self, date: Date) {
        self.access_date = date.encode();
    }

    fn set_modified(&mut self, date_time: DateTime) {
        self.modify_date = date_time.date.encode();
        self.modify_time = date_time.time.encode().0;
    }

    #[cfg(feature = "chrono")]
    pub(crate) fn reset_created(&mut self) {
        let now = DateTime::from(chrono::Local::now());
        self.set_created(now);
    }

    #[cfg(feature = "chrono")]
    pub(crate) fn reset_accessed(&mut self) -> bool {
        let now = Date::from(chrono::Local::now().date());
        if now == self.accessed() {
            false
        } else {
            self.set_accessed(now);
            true
        }
    }

    #[cfg(feature = "chrono")]
    pub(crate) fn reset_modified(&mut self) {
        let now = DateTime::from(chrono::Local::now());
        self.set_modified(now);
    }

    #[cfg(not(feature = "chrono"))]
    pub(crate) fn reset_created(&mut self) {
        // nop - user controls timestamps manually
    }

    #[cfg(not(feature = "chrono"))]
    pub(crate) fn reset_accessed(&mut self) -> bool {
        // nop - user controls timestamps manually
        false
    }

    #[cfg(not(feature = "chrono"))]
    pub(crate) fn reset_modified(&mut self) {
        // nop - user controls timestamps manually
    }

    pub(crate) fn serialize(&self, wrt: &mut Write) -> io::Result<()> {
        wrt.write_all(&self.name)?;
        wrt.write_u8(self.attrs.bits())?;
        wrt.write_u8(self.reserved_0)?;
        wrt.write_u8(self.create_time_0)?;
        wrt.write_u16::<LittleEndian>(self.create_time_1)?;
        wrt.write_u16::<LittleEndian>(self.create_date)?;
        wrt.write_u16::<LittleEndian>(self.access_date)?;
        wrt.write_u16::<LittleEndian>(self.first_cluster_hi)?;
        wrt.write_u16::<LittleEndian>(self.modify_time)?;
        wrt.write_u16::<LittleEndian>(self.modify_date)?;
        wrt.write_u16::<LittleEndian>(self.first_cluster_lo)?;
        wrt.write_u32::<LittleEndian>(self.size)?;
        Ok(())
    }

    pub(crate) fn is_deleted(&self) -> bool {
        self.name[0] == DIR_ENTRY_DELETED_FLAG
    }

    pub(crate) fn set_deleted(&mut self) {
        self.name[0] = DIR_ENTRY_DELETED_FLAG;
    }

    pub(crate) fn is_end(&self) -> bool {
        self.name[0] == 0
    }

    pub(crate) fn is_volume(&self) -> bool {
        self.attrs.contains(FileAttributes::VOLUME_ID)
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub(crate) struct DirLfnEntryData {
    order: u8,
    name_0: [u16; 5],
    attrs: FileAttributes,
    entry_type: u8,
    checksum: u8,
    name_1: [u16; 6],
    reserved_0: u16,
    name_2: [u16; 2],
}

impl DirLfnEntryData {
    pub(crate) fn new(order: u8, checksum: u8) -> Self {
        DirLfnEntryData {
            order, checksum,
            attrs: FileAttributes::LFN,
            ..Default::default()
        }
    }

    pub(crate) fn copy_name_from_slice(&mut self, lfn_part: &[u16; LFN_PART_LEN]) {
        self.name_0.copy_from_slice(&lfn_part[0..5]);
        self.name_1.copy_from_slice(&lfn_part[5..5+6]);
        self.name_2.copy_from_slice(&lfn_part[11..11+2]);
    }

    pub(crate) fn copy_name_to_slice(&self, lfn_part: &mut [u16]) {
        debug_assert!(lfn_part.len() == LFN_PART_LEN);
        lfn_part[0..5].copy_from_slice(&self.name_0);
        lfn_part[5..11].copy_from_slice(&self.name_1);
        lfn_part[11..13].copy_from_slice(&self.name_2);
    }

    pub(crate) fn serialize(&self, wrt: &mut Write) -> io::Result<()> {
        wrt.write_u8(self.order)?;
        for ch in self.name_0.iter() {
            wrt.write_u16::<LittleEndian>(*ch)?;
        }
        wrt.write_u8(self.attrs.bits())?;
        wrt.write_u8(self.entry_type)?;
        wrt.write_u8(self.checksum)?;
        for ch in self.name_1.iter() {
            wrt.write_u16::<LittleEndian>(*ch)?;
        }
        wrt.write_u16::<LittleEndian>(self.reserved_0)?;
        for ch in self.name_2.iter() {
            wrt.write_u16::<LittleEndian>(*ch)?;
        }
        Ok(())
    }

    #[cfg(feature = "alloc")]
    pub(crate) fn order(&self) -> u8 {
        self.order
    }

    #[cfg(feature = "alloc")]
    pub(crate) fn checksum(&self) -> u8 {
        self.checksum
    }

    pub(crate) fn is_deleted(&self) -> bool {
        self.order == DIR_ENTRY_DELETED_FLAG
    }

    pub(crate) fn set_deleted(&mut self) {
        self.order = DIR_ENTRY_DELETED_FLAG;
    }

    pub(crate) fn is_end(&self) -> bool {
        self.order == 0
    }
}

#[derive(Clone, Debug)]
pub(crate) enum DirEntryData {
    File(DirFileEntryData),
    Lfn(DirLfnEntryData),
}

impl DirEntryData {
    pub(crate) fn serialize(&self, wrt: &mut Write) -> io::Result<()> {
        match self {
            &DirEntryData::File(ref file) => file.serialize(wrt),
            &DirEntryData::Lfn(ref lfn) => lfn.serialize(wrt),
        }
    }

    pub(crate) fn deserialize(rdr: &mut Read) -> io::Result<Self> {
        let mut name = [0; 11];
        match rdr.read_exact(&mut name) {
            Err(ref err) if err.kind() == io::ErrorKind::UnexpectedEof => {
                // entries can occupy all clusters of directory so there is no zero entry at the end
                // handle it here by returning non-existing empty entry
                return Ok(DirEntryData::File(DirFileEntryData {
                    ..Default::default()
                }));
            }
            Err(err) => return Err(err),
            _ => {},
        }
        let attrs = FileAttributes::from_bits_truncate(rdr.read_u8()?);
        if attrs & FileAttributes::LFN == FileAttributes::LFN {
            // read long name entry
            let mut data = DirLfnEntryData {
                attrs, ..Default::default()
            };
            // use cursor to divide name into order and LFN name_0
            let mut cur = Cursor::new(&name);
            data.order = cur.read_u8()?;
            cur.read_u16_into::<LittleEndian>(&mut data.name_0)?;
            data.entry_type = rdr.read_u8()?;
            data.checksum = rdr.read_u8()?;
            rdr.read_u16_into::<LittleEndian>(&mut data.name_1)?;
            data.reserved_0 = rdr.read_u16::<LittleEndian>()?;
            rdr.read_u16_into::<LittleEndian>(&mut data.name_2)?;
            Ok(DirEntryData::Lfn(data))
        } else {
            // read short name entry
            let data = DirFileEntryData {
                name,
                attrs,
                reserved_0:       rdr.read_u8()?,
                create_time_0:    rdr.read_u8()?,
                create_time_1:    rdr.read_u16::<LittleEndian>()?,
                create_date:      rdr.read_u16::<LittleEndian>()?,
                access_date:      rdr.read_u16::<LittleEndian>()?,
                first_cluster_hi: rdr.read_u16::<LittleEndian>()?,
                modify_time:      rdr.read_u16::<LittleEndian>()?,
                modify_date:      rdr.read_u16::<LittleEndian>()?,
                first_cluster_lo: rdr.read_u16::<LittleEndian>()?,
                size:             rdr.read_u32::<LittleEndian>()?,
            };
            Ok(DirEntryData::File(data))
        }
    }

    pub(crate) fn is_deleted(&self) -> bool {
        match self {
            &DirEntryData::File(ref file) => file.is_deleted(),
            &DirEntryData::Lfn(ref lfn) => lfn.is_deleted(),
        }
    }

    pub(crate) fn set_deleted(&mut self) {
        match self {
            &mut DirEntryData::File(ref mut file) => file.set_deleted(),
            &mut DirEntryData::Lfn(ref mut lfn) => lfn.set_deleted(),
        }
    }

    pub(crate) fn is_end(&self) -> bool {
        match self {
            &DirEntryData::File(ref file) => file.is_end(),
            &DirEntryData::Lfn(ref lfn) => lfn.is_end(),
        }
    }
}

/// A DOS compatible date.
///
/// Used by `DirEntry` time-related methods.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct Date {
    /// Full year - [1980, 2107]
    pub year: u16,
    /// Month of the year - [1, 12]
    pub month: u16,
    /// Day of the month - [1, 31]
    pub day: u16,
}

impl Date {
    pub(crate) fn decode(dos_date: u16) -> Self {
        let (year, month, day) = ((dos_date >> 9) + 1980, (dos_date >> 5) & 0xF, dos_date & 0x1F);
        Date { year, month, day }
    }

    fn encode(&self) -> u16 {
        ((self.year - 1980) << 9) | (self.month << 5) | self.day
    }
}

/// A DOS compatible time.
///
/// Used by `DirEntry` time-related methods.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct Time {
    /// Hours after midnight - [0, 23]
    pub hour: u16,
    /// Minutes after the hour - [0, 59]
    pub min: u16,
    /// Seconds after the minute - [0, 59]
    pub sec: u16,
    /// Milliseconds after the second - [0, 999]
    pub millis: u16,
}

impl Time {
    pub(crate) fn decode(dos_time: u16, dos_time_hi_res: u8) -> Self {
        let hour = dos_time >> 11;
        let min = (dos_time >> 5) & 0x3F;
        let sec = (dos_time & 0x1F) * 2 + (dos_time_hi_res as u16) / 2;
        let millis = (dos_time_hi_res as u16 % 100) * 10;
        Time { hour, min, sec, millis }
    }

    fn encode(&self) -> (u16, u8) {
        let dos_time = (self.hour << 11) | (self.min << 5) | (self.sec / 2);
        let dos_time_hi_res = ((self.millis / 100) + (self.sec % 2) * 100) as u8;
        (dos_time, dos_time_hi_res)
    }
}

/// A DOS compatible date and time.
///
/// Used by `DirEntry` time-related methods.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct DateTime {
    /// A date part
    pub date: Date,
    // A time part
    pub time: Time,
}

impl DateTime {
    pub(crate) fn decode(dos_date: u16, dos_time: u16, dos_time_hi_res: u8) -> Self {
        DateTime {
            date: Date::decode(dos_date),
            time: Time::decode(dos_time, dos_time_hi_res),
        }
    }
}

#[cfg(feature = "chrono")]
impl From<Date> for chrono::Date<Local> {
    fn from(date: Date) -> Self {
        Local.ymd(date.year as i32, date.month as u32, date.day as u32)
    }
}

#[cfg(feature = "chrono")]
impl From<DateTime> for chrono::DateTime<Local> {
    fn from(date_time: DateTime) -> Self {
        chrono::Date::<Local>::from(date_time.date)
            .and_hms_milli(date_time.time.hour as u32, date_time.time.min as u32,
                date_time.time.sec as u32, date_time.time.millis as u32)
    }
}

#[cfg(feature = "chrono")]
impl From<chrono::Date<Local>> for Date {
    fn from(date: chrono::Date<Local>) -> Self {
        Date {
            year: date.year() as u16,
            month: date.month() as u16,
            day: date.day() as u16,
        }
    }
}

#[cfg(feature = "chrono")]
impl From<chrono::DateTime<Local>> for DateTime {
    fn from(date_time: chrono::DateTime<Local>) -> Self {
        DateTime {
            date: Date::from(date_time.date()),
            time: Time {
                hour: date_time.hour() as u16,
                min: date_time.minute() as u16,
                sec: date_time.second() as u16,
                millis: (date_time.nanosecond() / 1_000_000) as u16,
            },
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DirEntryEditor {
    data: DirFileEntryData,
    pos: u64,
    dirty: bool,
}

impl DirEntryEditor {
    fn new(data: DirFileEntryData, pos: u64) -> Self {
        DirEntryEditor {
            data, pos,
            dirty: false,
        }
    }

    pub(crate) fn inner(&self) -> &DirFileEntryData {
        &self.data
    }

    pub(crate) fn set_first_cluster(&mut self, first_cluster: Option<u32>, fat_type: FatType) {
        if first_cluster != self.data.first_cluster(fat_type) {
            self.data.set_first_cluster(first_cluster, fat_type);
            self.dirty = true;
        }
    }

    pub(crate) fn set_size(&mut self, size: u32) {
        match self.data.size() {
            Some(n) if size != n => {
                self.data.set_size(size);
                self.dirty = true;
            },
            _ => {},
        }
    }

    pub(crate) fn set_created(&mut self, date_time: DateTime) {
        if date_time != self.data.created() {
            self.data.set_created(date_time);
            self.dirty = true;
        }
    }

    pub(crate) fn set_accessed(&mut self, date: Date) {
        if date != self.data.accessed() {
            self.data.set_accessed(date);
            self.dirty = true;
        }
    }

    pub(crate) fn set_modified(&mut self, date_time: DateTime) {
        if date_time != self.data.modified() {
            self.data.set_modified(date_time);
            self.dirty = true;
        }
    }

    pub(crate) fn reset_accessed(&mut self) {
        self.dirty |= self.data.reset_accessed();
    }

    pub(crate) fn reset_modified(&mut self) {
        self.data.reset_modified();
        self.dirty = true;
    }

    pub(crate) fn flush<T: ReadWriteSeek>(&mut self, fs: &FileSystem<T>) -> io::Result<()> {
        if self.dirty {
            self.write(fs)?;
            self.dirty = false;
        }
        Ok(())
    }

    fn write<T: ReadWriteSeek>(&self, fs: &FileSystem<T>) -> io::Result<()> {
        let mut disk = fs.disk.borrow_mut();
        disk.seek(io::SeekFrom::Start(self.pos))?;
        self.data.serialize(&mut *disk)
    }
}

/// A FAT directory entry.
///
/// `DirEntry` is returned by `DirIter` when reading a directory.
#[derive(Clone)]
pub struct DirEntry<'a, T: ReadWriteSeek + 'a> {
    pub(crate) data: DirFileEntryData,
    pub(crate) short_name: ShortName,
    #[cfg(feature = "alloc")]
    pub(crate) lfn: Vec<u16>,
    pub(crate) entry_pos: u64,
    pub(crate) offset_range: (u64, u64),
    pub(crate) fs: &'a FileSystem<T>,
}

impl <'a, T: ReadWriteSeek> DirEntry<'a, T> {
    /// Returns short file name.
    ///
    /// Non-ASCII characters are replaced by the replacement character (U+FFFD).
    #[cfg(feature = "alloc")]
    pub fn short_file_name(&self) -> String {
        self.short_name.to_string()
    }

    /// Returns short file name as byte array slice.
    ///
    /// Characters are encoded in the OEM codepage.
    pub fn short_file_name_as_bytes(&self) -> &[u8] {
        self.short_name.as_bytes()
    }

    /// Returns long file name or if it doesn't exist fallbacks to short file name.
    #[cfg(feature = "alloc")]
    pub fn file_name(&self) -> String {
        if self.lfn.len() > 0 {
            String::from_utf16_lossy(&self.lfn)
        } else {
            self.data.lowercase_name()
        }
    }

    /// Returns file attributes.
    pub fn attributes(&self) -> FileAttributes {
        self.data.attrs
    }

    /// Checks if entry belongs to directory.
    pub fn is_dir(&self) -> bool {
        self.data.is_dir()
    }

    /// Checks if entry belongs to regular file.
    pub fn is_file(&self) -> bool {
        self.data.is_file()
    }

    pub(crate) fn first_cluster(&self) -> Option<u32> {
        self.data.first_cluster(self.fs.fat_type())
    }

    fn editor(&self) -> DirEntryEditor {
        DirEntryEditor::new(self.data.clone(), self.entry_pos)
    }

    pub(crate) fn is_same_entry(&self, other: &DirEntry<T>) -> bool {
        self.entry_pos == other.entry_pos
    }

    /// Returns `File` struct for this entry.
    ///
    /// Panics if this is not a file.
    pub fn to_file(&self) -> File<'a, T> {
        assert!(!self.is_dir(), "Not a file entry");
        File::new(self.first_cluster(), Some(self.editor()), self.fs)
    }

    /// Returns `Dir` struct for this entry.
    ///
    /// Panics if this is not a directory.
    pub fn to_dir(&self) -> Dir<'a, T> {
        assert!(self.is_dir(), "Not a directory entry");
        match self.first_cluster() {
            Some(n) => {
                let file = File::new(Some(n), Some(self.editor()), self.fs);
                Dir::new(DirRawStream::File(file), self.fs)
            },
            None => self.fs.root_dir(),
        }
    }

    /// Returns file size or 0 for directory.
    pub fn len(&self) -> u64 {
        self.data.size as u64
    }

    /// Returns file creation date and time.
    ///
    /// Resolution of the time field is 1/100s.
    pub fn created(&self) -> DateTime {
        self.data.created()
    }

    /// Returns file last access date.
    pub fn accessed(&self) -> Date {
        self.data.accessed()
    }

    /// Returns file last modification date and time.
    ///
    /// Resolution of the time field is 2s.
    pub fn modified(&self) -> DateTime {
        self.data.modified()
    }

    pub(crate) fn raw_short_name(&self) -> &[u8; 11] {
        &self.data.name
    }

    #[cfg(feature = "alloc")]
    pub(crate) fn eq_name(&self, name: &str) -> bool {
        self.file_name().eq_ignore_ascii_case(name) || self.short_name.eq_ignore_ascii_case(name)
    }
    #[cfg(not(feature = "alloc"))]
    pub(crate) fn eq_name(&self, name: &str) -> bool {
        self.short_name.eq_ignore_ascii_case(name)
    }
}

impl <'a, T: ReadWriteSeek> fmt::Debug for DirEntry<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        self.data.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_name_with_ext() {
        let mut raw_short_name = [0u8;11];
        raw_short_name.copy_from_slice("FOO     BAR".as_bytes());
        assert_eq!(ShortName::new(&raw_short_name).to_string(), "FOO.BAR");
        raw_short_name.copy_from_slice("LOOK AT M E".as_bytes());
        assert_eq!(ShortName::new(&raw_short_name).to_string(), "LOOK AT.M E");
    }

    #[test]
    fn short_name_without_ext() {
        let mut raw_short_name = [0u8;11];
        raw_short_name.copy_from_slice("FOO        ".as_bytes());
        assert_eq!(ShortName::new(&raw_short_name).to_string(), "FOO");
        raw_short_name.copy_from_slice("LOOK AT    ".as_bytes());
        assert_eq!(ShortName::new(&raw_short_name).to_string(), "LOOK AT");
    }

    #[test]
    fn short_name_05_changed_to_e5() {
        let raw_short_name = [0x05;11];
        assert_eq!(ShortName::new(&raw_short_name).as_bytes(), [0xE5, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, '.' as u8, 0x05, 0x05, 0x05]);
    }

    #[test]
    fn lowercase_short_name() {
        let mut raw_short_name = [0u8;11];
        raw_short_name.copy_from_slice("FOO     RS ".as_bytes());
        let mut raw_entry = DirFileEntryData {
            name: raw_short_name,
            reserved_0: (1 << 3) | (1 << 4),
            ..Default::default()
        };
        assert_eq!(raw_entry.lowercase_name(), "foo.rs");
        raw_entry.reserved_0 = 1 << 3;
        assert_eq!(raw_entry.lowercase_name(), "foo.RS");
        raw_entry.reserved_0 = 1 << 4;
        assert_eq!(raw_entry.lowercase_name(), "FOO.rs");
        raw_entry.reserved_0 = 0;
        assert_eq!(raw_entry.lowercase_name(), "FOO.RS");
    }
}
