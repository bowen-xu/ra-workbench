//! Fixed SyncTeX operations for already identity-checked Agent compile results.
//! Application composition selects the writing adapter. This port exposes no
//! database, source selection, process command or toolchain configuration API.
pub(crate) use research_workbench_native_sdk::writing_compile::{
    SynctexPdfLocation, SynctexSourceLocation,
};
use std::path::Path;

/// The caller has already checked artifact identity and selected either its
/// restored workspace or its retained source snapshot. Toolchain JSON remains
/// opaque to Core and is reused unchanged for this query.
pub(crate) struct AgentCompileSynctexQuery<'a> {
    pub(crate) toolchain_json: &'a str,
    pub(crate) runtime_home: &'a Path,
    pub(crate) snapshot: &'a Path,
    pub(crate) output_directory: &'a Path,
    pub(crate) pdf: &'a Path,
}

type ForwardQuery =
    fn(&AgentCompileSynctexQuery<'_>, &str, u32, u32) -> Result<Vec<SynctexPdfLocation>, String>;
type InverseQuery =
    fn(&AgentCompileSynctexQuery<'_>, u32, f64, f64) -> Result<Vec<SynctexSourceLocation>, String>;

#[derive(Clone, Copy)]
pub(crate) struct AgentCompileSynctex {
    pub(crate) prepare_toolchain: fn(String, &Path) -> Result<String, String>,
    pub(crate) forward: ForwardQuery,
    pub(crate) inverse: InverseQuery,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_locations_preserve_the_existing_ipc_shape() {
        assert_eq!(
            serde_json::to_value(SynctexPdfLocation {
                page: 2,
                x: 12.5,
                y: 44.0,
                width: 18.0,
                height: 9.0,
            })
            .unwrap(),
            serde_json::json!({"page":2,"x":12.5,"y":44.0,"width":18.0,"height":9.0})
        );
    }

    #[test]
    fn source_locations_preserve_camel_case_and_nullable_column() {
        for column in [Some(3), None] {
            assert_eq!(
                serde_json::to_value(SynctexSourceLocation {
                    relative_path: "sections/a.tex".into(),
                    line: 7,
                    column,
                })
                .unwrap(),
                serde_json::json!({"relativePath":"sections/a.tex","line":7,"column":column})
            );
        }
    }
}
