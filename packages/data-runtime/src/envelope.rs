//! Versioned protobuf boundary. Generated code stays in Cargo's output folder.
// Pinned protobuf 2's generated syntax predates current Rust lint defaults.
// Suppress only in generated output; handwritten code retains strict checks.
#[allow(
    renamed_and_removed_lints,
    unused_parens,
    mismatched_lifetime_syntaxes,
    clippy::all
)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/mod.rs"));
}

pub use generated::log::{LogEnvelope, LogState};
pub const LOG_SCHEMA_VERSION: u32 = 1;

pub struct EnvelopeIndex;
impl raft_engine::MessageExt for EnvelopeIndex {
    type Entry = LogEnvelope;
    fn index(entry: &Self::Entry) -> u64 {
        entry.position
    }
}
