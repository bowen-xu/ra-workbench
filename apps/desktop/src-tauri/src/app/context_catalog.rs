//! Empty Core catalog ports before independent domain bindings arrive.
use crate::agent::context_catalog::ContextCatalogSource;
pub(crate) fn source() -> ContextCatalogSource {
    let source = ContextCatalogSource::default();

    source
}
