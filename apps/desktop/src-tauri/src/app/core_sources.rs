//! Core-owned startup sources. Independent packages supply their finite domain bindings.

pub(crate) fn sources() -> crate::storage::AppStoreSources {
    crate::storage::AppStoreSources {
        context_catalog_source: crate::context_catalog_app::source(),
        ..Default::default()
    }
}
