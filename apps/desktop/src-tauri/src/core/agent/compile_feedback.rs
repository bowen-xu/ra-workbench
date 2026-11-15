//! Fixed formal manuscript compile reads. The host retains authorization,
//! workers and both database guard lifetimes; writing owns history and SyncTeX.
use super::compile_synctex::SynctexPdfLocation;
use crate::agent::writing_compile_contracts::{
    CompileRun, FormalSynctexRequest, SynctexRunContext,
};
use rusqlite::Connection;
use std::path::Path;
use uuid::Uuid;

type Latest = fn(&Connection, Uuid) -> Result<Option<CompileRun>, String>;
type ValidateForward = fn(&str, &str, u32, u32) -> Result<FormalSynctexRequest, String>;
type ForwardContext = fn(&Connection, &str) -> Result<SynctexRunContext, String>;
type Forward = fn(
    &Path,
    &Path,
    &FormalSynctexRequest,
    &SynctexRunContext,
) -> Result<Vec<SynctexPdfLocation>, String>;

#[derive(Clone, Copy)]
pub(crate) struct WritingCompileFeedbackSource {
    pub latest: Latest,
    pub latest_pdf: Latest,
    pub validate_forward: ValidateForward,
    pub forward_context: ForwardContext,
    pub forward: Forward,
}
