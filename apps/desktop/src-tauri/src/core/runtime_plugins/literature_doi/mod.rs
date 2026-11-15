//! DOI preview persistence at the original import-job and autocommit positions.
mod resources;
pub(crate) mod storage;

pub(crate) use resources::DoiResources;

#[cfg(test)]
mod tests;
