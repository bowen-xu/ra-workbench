//! Short Core read, lock-free native decision, then original transaction consume.
use super::{Error, SelectedRequest, SourceReceipt, native_diagnostic, source};
use crate::{
    runtime_plugins::literature_trace::data::{
        PreparedTrace, capture_note_trace_in, consume_prepared_trace_in,
    },
    storage::AppStore,
};
use research_agent_core::submission::ContextText;
use research_workbench_native_sdk::literature_trace_protocol::TraceDecision;
use rusqlite::Transaction;

pub(super) struct PreparedNativeTrace {
    data: PreparedTrace,
    decision: TraceDecision,
    receipt: SourceReceipt,
    package_key: String,
    instance_id: String,
    native_request_id: String,
}

impl PreparedNativeTrace {
    /// The caller has just repeated binding, Note/full context and selection
    /// checks in its original Immediate transaction. No child wait occurs here.
    pub(super) fn consume_in(&self, tx: &Transaction<'_>) -> Result<ContextText, Error> {
        consume_prepared_trace_in(tx, &self.data, &self.decision).map_err(|cause| {
            let mut error = native_diagnostic::consumption(self.receipt, &self.package_key, cause);
            if let Error::Native(diagnostic) = &mut error {
                diagnostic.instance_id = Some(self.instance_id.clone());
                diagnostic.native_request_id = Some(self.native_request_id.clone());
            }
            error
        })
    }
}

pub(super) async fn prepare(
    store: &AppStore,
    request: &SelectedRequest,
    receipt: SourceReceipt,
) -> Result<Option<PreparedNativeTrace>, Error> {
    let (Some(binding), Some(retrieval), source::Selection::Page(page)) = (
        store.native_note_trace(),
        request.retrieval.as_ref(),
        &request.context,
    ) else {
        return Ok(None);
    };
    let binding = binding.clone();
    let key = binding.package_key.clone();
    let preflight_page = page.clone();
    let preflight_retrieval = retrieval.clone();
    let preflight_input = request.user_input.clone();
    let preflight_store = store.clone();
    // Preserve Note/full-context then selection error priority even when the
    // configured package is missing. Capture repeats these reads after resource
    // acceptance, because no database guard may span that asynchronous work.
    tokio::task::spawn_blocking(move || {
        let mut db = preflight_store
            .connection()
            .map_err(|_| Error::Unavailable)?;
        let tx = db.transaction().map_err(|_| Error::Unavailable)?;
        receipt.check_binding(&tx)?;
        drop(source::materialize_contexts(
            &tx,
            &source::Selection::Page(preflight_page),
            &preflight_input,
        )?);
        preflight_retrieval
            .validate()
            .map_err(|_| Error::InvalidInput)?;
        tx.commit().map_err(|_| Error::Unavailable)
    })
    .await
    .map_err(|_| Error::Unavailable)??;
    let resources = binding
        .service
        .accept(&key)
        .await
        .map_err(|cause| native_diagnostic::host(receipt, &key, *cause))?;
    let selected = source::Selection::Page(page.clone());
    let retrieval = retrieval.clone();
    let user_input = request.user_input.clone();
    let capture_store = store.clone();
    let capture_key = key.clone();
    let data = tokio::task::spawn_blocking(move || {
        let mut db = capture_store.connection().map_err(|_| Error::Unavailable)?;
        let tx = db.transaction().map_err(|_| Error::Unavailable)?;
        receipt.check_binding(&tx)?;
        // Discard this local projection; T1 materializes the original frozen
        // Note selection again. Its body is never sent to the literature host.
        drop(source::materialize_contexts(&tx, &selected, &user_input)?);
        retrieval.validate().map_err(|_| Error::InvalidInput)?;
        let data = capture_note_trace_in(
            &tx,
            resources,
            &retrieval,
            &receipt.resource.id().to_string(),
        )
        .map_err(|cause| native_diagnostic::consumption(receipt, &capture_key, cause))?;
        tx.commit().map_err(|_| Error::Unavailable)?;
        Ok::<_, Error>(data)
    })
    .await
    .map_err(|_| Error::Unavailable)??;
    // `data` is owned: the capture closure and all guards have ended. The
    // service removes its private input before returning this finite decision.
    let input = data
        .note_input()
        .map_err(|cause| native_diagnostic::consumption(receipt, &key, cause))?;
    let decision = binding
        .service
        .decide(key.clone(), input)
        .await
        .map_err(|cause| native_diagnostic::host(receipt, &key, *cause))?;
    Ok(Some(PreparedNativeTrace {
        data,
        decision: decision.decision,
        receipt,
        package_key: key,
        instance_id: decision.instance_id,
        native_request_id: decision.request_id,
    }))
}
