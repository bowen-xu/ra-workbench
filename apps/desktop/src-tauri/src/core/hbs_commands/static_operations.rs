//! Original synchronous path, owned by a blocking worker from async IPC.
use super::*;

pub(super) fn commit(
    store: AppStore,
    kind: String,
    resource_id: String,
    request: hbs_storage::Commit,
) -> Result<Value, String> {
    let mut db = store.connection().map_err(|error| error.to_string())?;
    let result = hbs_storage::commit(
        store.generation_policy_source(),
        store.paper_reading_source(),
        &mut db,
        &kind,
        &resource_id,
        request,
    )?;
    let paper = if kind == "structured" {
        crate::structured_resources::get(&db, &resource_id)
            .map_err(|error| error.to_string())?
            .paper_id
    } else {
        None
    };
    drop(db);
    if kind == "structured" && result["type"] == "receipt" {
        store.notify_structured_resource_changed(&resource_id, paper.as_deref());
    }
    Ok(result)
}
