//! One owned persistence runtime. Business authorization and mutation codecs
//! stay with callers; the log does not execute SQL or external side effects.
mod codec_counters;
pub mod config;
pub mod coordinator;
pub mod envelope;
pub mod error;
mod extent;
mod io_counters;
pub mod log;

pub use codec_counters::{CodecCounters, CodecSnapshot};
pub use config::LogConfig;
pub use coordinator::{
    Accepted, Admission, AdmissionGuard, Coordinator, CoordinatorConfig, CoordinatorError,
    Durability, InspectionGuard, InspectionSnapshot, OperationReceipt, ProjectionState, Projector,
    ReadFence, ReclaimFence, WriteMode, WriteOptions, WriteReceipt,
};
pub use error::{LogError, LogResult};
pub use extent::ExtentUsage;
pub use io_counters::{LogIoCounters, LogIoSnapshot};
pub use log::{LogHead, LogRecord, Maintenance, RaftLog};
