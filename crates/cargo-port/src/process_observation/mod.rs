//! Process identity evidence used to bind owned-run control to a live process.

pub(crate) mod identity;

mod disk_reads;
pub(crate) use disk_reads::DiskReadBytes;
pub(crate) use disk_reads::DiskReadMeasurement;
