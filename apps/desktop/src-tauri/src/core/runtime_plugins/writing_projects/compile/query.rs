//! The original fixed query IO followed by the selected writing parser.
//! Callers retain argument validation and their formal/result identity boundary.
use super::{domain::Domain, synctex_io};
use crate::{
    agent::compile_synctex::{AgentCompileSynctexQuery, SynctexPdfLocation, SynctexSourceLocation},
    runtime_plugins::native::NativeResult,
};
use research_workbench_native_sdk::writing_compile::{Decision, Input};

pub(super) fn forward(
    domain: &Domain,
    query: &AgentCompileSynctexQuery<'_>,
    relative_path: &str,
    line: u32,
    column: u32,
) -> NativeResult<Vec<SynctexPdfLocation>> {
    let raw_output = synctex_io::forward_output(
        query.runtime_home,
        query.toolchain_json,
        query.snapshot,
        query.output_directory,
        query.pdf,
        relative_path,
        line,
        column,
    )
    .map_err(|cause| domain.error(cause))?;
    match domain.call(Input::ForwardOutput { raw_output })? {
        Decision::ForwardLocations { locations } => {
            Ok(locations.into_iter().map(Into::into).collect())
        }
        _ => Err(domain.mismatch()),
    }
}

pub(super) fn inverse(
    domain: &Domain,
    query: &AgentCompileSynctexQuery<'_>,
    page: u32,
    x: f64,
    y: f64,
) -> NativeResult<Vec<SynctexSourceLocation>> {
    let raw_output = synctex_io::inverse_output(
        query.runtime_home,
        query.toolchain_json,
        query.output_directory,
        query.pdf,
        page,
        x,
        y,
    )
    .map_err(|cause| domain.error(cause))?;
    let candidates = match domain.call(Input::InverseCandidates { raw_output })? {
        Decision::InverseCandidates { candidates } => candidates,
        _ => return Err(domain.mismatch()),
    };
    let resolved = synctex_io::resolve_inputs(query.snapshot, candidates.items)
        .map_err(|cause| domain.error(cause))?;
    match domain.call(Input::InverseResolved {
        locations: resolved.locations,
        invalid_input: resolved.invalid_input,
        generated_input: candidates.generated_input,
    })? {
        Decision::InverseLocations { locations } => Ok(locations),
        _ => Err(domain.mismatch()),
    }
}
