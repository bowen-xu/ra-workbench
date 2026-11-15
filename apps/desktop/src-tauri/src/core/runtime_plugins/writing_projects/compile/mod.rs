//! Original formal compile adaptation, selected by its writing package.
pub(crate) mod artifacts;
pub(crate) mod cache;
mod domain;
pub(crate) mod execution;
pub(crate) mod export;
pub(crate) mod feedback;
pub(crate) mod process;
mod query;
pub(crate) mod read;
pub(crate) mod repository;
pub(crate) mod resources;
pub(crate) mod snapshot;
pub(crate) mod start;
pub(crate) mod synctex;
pub(crate) mod synctex_io;
pub(crate) mod tools;
pub(crate) mod workspace;
#[cfg(target_os = "macos")]
pub(crate) mod workspace_recipe;

#[cfg(test)]
mod tests;
