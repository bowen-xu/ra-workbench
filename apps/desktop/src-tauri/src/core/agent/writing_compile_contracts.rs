//! Pure values shared by Core compile ports and the built-in writing provider.
//! Domain validation and execution stay with their existing owners.
pub(crate) use research_workbench_native_sdk::writing_compile::{
    CompileArtifact, CompileDiagnostic, CompileRun, LatexTool, LatexToolchainInfo,
};

pub(crate) struct SynctexRunContext {
    pub(crate) manuscript_id: String,
    pub(crate) root_path: String,
    pub(crate) toolchain_json: String,
    pub(crate) input_manifest_hash: String,
}

/// Validated formal-run input. Core preserves this opaque value between the two
/// database reads; run identity remains the original string, never normalized.
#[expect(
    dead_code,
    reason = "Retained formal-run DTO for the Core compile feedback port."
)]
pub(crate) struct FormalSynctexRequest {
    pub(crate) run_id: String,
    pub(crate) relative_path: std::path::PathBuf,
    pub(crate) line: u32,
    pub(crate) column: u32,
}

#[cfg(target_os = "macos")]
pub(crate) struct LatexWorkspaceCompileRequest<'a> {
    pub root_path: &'a str,
    pub engine: &'a str,
    pub draft: bool,
    pub stop_on_first_error: bool,
    pub from_scratch: bool,
    pub output_directory: &'a std::path::Path,
}

#[cfg(target_os = "macos")]
pub(crate) struct LatexWorkspaceCompileRecipe {
    pub program: std::path::PathBuf,
    pub arguments: Vec<std::ffi::OsString>,
    pub path: String,
    pub read_only: Vec<std::path::PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn toolchain_keeps_original_camel_case_and_nullable_tool_fields() {
        let value = serde_json::json!({"available":true,"tools":[{"name":"latexmk","available":true,"path":"/synthetic/latexmk","version":null}],"supportedEngines":["pdflatex"],"checkedAt":"frozen"});
        let parsed: LatexToolchainInfo = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), value);
    }
    #[test]
    fn diagnostics_keep_original_fields_and_nullable_locations() {
        for (file, line, column) in [
            (None, None, None),
            (Some("main.tex".into()), Some(5), Some(3)),
        ] {
            let diagnostic = CompileDiagnostic {
                severity: "error".into(),
                message: "synthetic".into(),
                file: file.clone(),
                line,
                column,
            };
            assert_eq!(
                serde_json::to_value(diagnostic).unwrap(),
                serde_json::json!({"severity":"error","message":"synthetic","file":file,"line":line,"column":column})
            );
        }
    }
}
