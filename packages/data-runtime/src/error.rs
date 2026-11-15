use std::io;

pub type LogResult<T> = Result<T, LogError>;

#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error(
        "log_capacity_not_accepted: used={used}, reserved={reserved}, requested={requested}, limit={limit}"
    )]
    CapacityNotAccepted {
        used: u64,
        reserved: u64,
        requested: u64,
        limit: u64,
    },
    #[error("log_invalid_argument: {0}")]
    InvalidArgument(&'static str),
    #[error("log_epoch_mismatch: storage belongs to another epoch")]
    EpochMismatch,
    #[error("log_corrupt: {0}")]
    Corrupt(&'static str),
    #[error("log_compacted: requested prefix is no longer retained")]
    Compacted,
    #[error("log_poisoned: drop and reopen the storage owner before continuing")]
    Poisoned,
    #[error("log_engine_panic: {operation} outcome is unknown; owner is poisoned")]
    EnginePanic { operation: &'static str },
    #[error("log_engine_error: {operation} failed; owner is poisoned: {source}")]
    Engine {
        operation: &'static str,
        #[source]
        source: raft_engine::Error,
    },
    #[error("log_io_error: {operation}: {source}")]
    Io {
        operation: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("log_record_too_large: record exceeds the read byte limit")]
    RecordTooLarge,
    #[error("log_encoding_error: {operation}: {source}")]
    Encoding {
        operation: &'static str,
        #[source]
        source: raft_engine::Error,
    },
}
