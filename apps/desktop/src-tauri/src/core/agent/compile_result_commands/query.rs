//! Fixed source selection after the original retained-result identity checks.
use super::*;
use crate::runtime_plugins::{
    native::NativeHostError, writing_projects::compile::workspace::AgentCompileSource,
};

pub(super) enum Query {
    Static(AgentCompileSynctex),
    Runtime(AgentCompileSource),
}

fn native_failure(error: Box<NativeHostError>) -> String {
    serde_json::to_string(&error).unwrap_or_else(|_| "synctex_unavailable".into())
}

impl Query {
    pub(super) fn select(
        store: &AppStore,
        source: Option<AgentCompileSynctex>,
        runtime: tokio::runtime::Handle,
    ) -> Result<Self, String> {
        match source {
            Some(source) => Ok(Self::Static(source)),
            None => {
                let binding = store
                    .native_writing_context()
                    .cloned()
                    .ok_or("synctex_unavailable")?;
                AgentCompileSource::new(binding, runtime)
                    .map(Self::Runtime)
                    .map_err(native_failure)
            }
        }
    }

    pub(super) fn prepare(&self, store: &AppStore) -> Result<(String, std::path::PathBuf), String> {
        match self {
            Self::Static(source) => prepare_synctex_toolchain(store, *source),
            Self::Runtime(source) => {
                let info = source.detect(store).map_err(native_failure)?;
                let json = serde_json::to_string(&info).map_err(|_| "synctex_unavailable")?;
                Ok((json, store.writing_cache_directory().join("toolchain-home")))
            }
        }
    }

    pub(super) fn forward(
        &self,
        request: &AgentCompileSynctexQuery<'_>,
        path: &str,
        line: u32,
        column: u32,
    ) -> Result<Vec<SynctexPdfLocation>, String> {
        match self {
            Self::Static(source) => (source.forward)(request, path, line, column),
            Self::Runtime(source) => source
                .forward(request, path, line, column)
                .map_err(native_failure),
        }
    }

    pub(super) fn inverse(
        &self,
        request: &AgentCompileSynctexQuery<'_>,
        page: u32,
        x: f64,
        y: f64,
    ) -> Result<Vec<SynctexSourceLocation>, String> {
        match self {
            Self::Static(source) => (source.inverse)(request, page, x, y),
            Self::Runtime(source) => source.inverse(request, page, x, y).map_err(native_failure),
        }
    }
}
