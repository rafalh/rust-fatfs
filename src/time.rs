#[cfg(feature = "chrono")]
use chrono;
#[cfg(feature = "chrono")]
use chrono::{Datelike, Local, TimeZone, Timelike};

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

    pub(crate) fn encode(&self) -> u16 {
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

    pub(crate) fn encode(&self) -> (u16, u8) {
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
        chrono::Date::<Local>::from(date_time.date).and_hms_milli(
            date_time.time.hour as u32,
            date_time.time.min as u32,
            date_time.time.sec as u32,
            date_time.time.millis as u32,
        )
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

/// A current time and date provider.
///
/// Provides a custom implementation for a time resolution used when updating directory entry time fields.
/// Default implementation gets time from `chrono` crate if `chrono` feature is enabled.
/// Otherwise default implementation returns DOS minimal date-time (1980/1/1 0:00:00).
pub trait TimeProvider {
    fn get_current_date(&self) -> Date;
    fn get_current_date_time(&self) -> DateTime;
}

#[derive(Clone)]
pub(crate) struct DefaultTimeProvider {
    _dummy: (),
}

impl TimeProvider for DefaultTimeProvider {
    #[cfg(feature = "chrono")]
    fn get_current_date(&self) -> Date {
        Date::from(chrono::Local::now().date())
    }
    #[cfg(not(feature = "chrono"))]
    fn get_current_date(&self) -> Date {
        Date::decode(0)
    }

    #[cfg(feature = "chrono")]
    fn get_current_date_time(&self) -> DateTime {
        DateTime::from(chrono::Local::now())
    }
    #[cfg(not(feature = "chrono"))]
    fn get_current_date_time(&self) -> DateTime {
        DateTime::decode(0, 0, 0)
    }
}

pub(crate) static DEFAULT_TIME_PROVIDER: DefaultTimeProvider = DefaultTimeProvider { _dummy: () };
