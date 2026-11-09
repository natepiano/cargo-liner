//! Borrowed fields from the kernel's Linux process stat record.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

use crate::constants::BIRTH_STAT_COMM_END;
use crate::constants::BIRTH_STAT_START_INDEX;
use crate::constants::CPU_STAT_TIME_FIELDS;
use crate::constants::CPU_STAT_TIME_INDEX;
use crate::constants::RESIDENT_PAGES_INDEX;

/// One boundary rule serves process births, discovery and CPU accounting.
/// Fields borrow the read buffer; parsing allocates no field table or strings.
pub(crate) struct LinuxProcessStat<'stat> {
    pid:    &'stat [u8],
    name:   &'stat OsStr,
    fields: [&'stat [u8]; RESIDENT_PAGES_INDEX + 1],
}

impl<'stat> LinuxProcessStat<'stat> {
    /// `comm` can contain parentheses, whitespace and arbitrary non-UTF-8 bytes.
    pub(crate) fn parse(bytes: &'stat [u8]) -> Result<Self, ()> {
        let start = bytes.iter().position(|byte| *byte == b'(').ok_or(())?;
        let end = bytes
            .iter()
            .rposition(|byte| *byte == BIRTH_STAT_COMM_END)
            .ok_or(())?;
        if start >= end {
            return Err(());
        }
        let mut fields = [b"".as_slice(); RESIDENT_PAGES_INDEX + 1];
        for (slot, field) in fields.iter_mut().zip(
            bytes[end + 1..]
                .split(u8::is_ascii_whitespace)
                .filter(|field| !field.is_empty()),
        ) {
            *slot = field;
        }
        Ok(Self {
            pid: bytes[..start].trim_ascii(),
            name: OsStr::from_bytes(&bytes[start + 1..end]),
            fields,
        })
    }

    fn number(bytes: &[u8]) -> Result<u64, ()> {
        if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
            return Err(());
        }
        std::str::from_utf8(bytes)
            .map_err(|_| ())?
            .parse()
            .map_err(|_| ())
    }

    pub(crate) fn pid(&self) -> Result<u32, ()> {
        u32::try_from(Self::number(self.pid)?).map_err(|_| ())
    }

    pub(crate) const fn name(&self) -> &'stat OsStr { self.name }

    pub(crate) fn parent(&self) -> Result<u32, ()> {
        u32::try_from(Self::number(self.fields[1])?).map_err(|_| ())
    }

    /// Discovery uses only this process's counters, without its waited children.
    pub(crate) fn own_ticks(&self) -> Result<u64, ()> {
        Ok(Self::number(self.fields[CPU_STAT_TIME_INDEX])?
            .saturating_add(Self::number(self.fields[CPU_STAT_TIME_INDEX + 1])?))
    }

    /// Invocation accounting includes all four counters and rejects overflow.
    pub(crate) fn cpu_ticks(&self) -> Result<u64, ()> {
        self.fields[CPU_STAT_TIME_INDEX..CPU_STAT_TIME_INDEX + CPU_STAT_TIME_FIELDS]
            .iter()
            .try_fold(0_u64, |sum, field| {
                sum.checked_add(Self::number(field)?).ok_or(())
            })
    }

    pub(crate) fn start_ticks(&self) -> Result<u64, ()> {
        Self::number(self.fields[BIRTH_STAT_START_INDEX])
    }

    pub(crate) fn resident_pages(&self) -> Result<u64, ()> {
        Self::number(self.fields[RESIDENT_PAGES_INDEX])
    }
}
