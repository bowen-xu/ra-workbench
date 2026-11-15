//! Original retrieval stage order over Core providers, vectors and fixed SQL.
use super::{
    capture,
    failure::Error,
    persist::{self, TraceWrite},
    resources::{ReadSet, RetrievalResources},
};
use crate::{
    ai::{
        CancellationSignal, EmbeddingAccess, PreparedEmbedding,
        knowledge_contracts::{ConceptRetrievalError as E, SemanticIndexError},
    },
    data_interface::guarded_transaction,
    hbs_storage::boundary::HbsOutcome,
    runtime_plugins::literature_trace::service::LiteratureTraceService,
    semantic_engine::{
        SemanticNamespace, SemanticVectorSearchHit, SemanticVectorStore, VectorAccess,
    },
    storage::AppStore,
};
use research_workbench_native_sdk::concept_retrieval::{
    ConceptRetrievalDecisionV1 as Decision, ConceptRetrievalInputV1 as Input,
    ConceptRetrievalPreview, ConceptRetrievalPreviewDraft, VectorTargetV1,
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;
#[cfg(test)]
mod tests;
type Result<T> = std::result::Result<T, Error>;
#[derive(Default)]
struct Progress {
    trace: Option<String>,
    lease_release_confirmed: Option<bool>,
    trace_write_started: bool,
}
struct State {
    store: AppStore,
    resources: Arc<RetrievalResources>,
    reads: ReadSet,
    trace: String,
    paper: Option<String>,
    query: String,
    fts: String,
    query_sha256: String,
    lease_release_confirmed: Option<bool>,
    trace_write_started: bool,
}
impl State {
    async fn read<T: Send + 'static>(
        mut self,
        operation: impl FnOnce(&mut Connection, &mut Self) -> Result<T> + Send + 'static,
    ) -> Result<(Self, Result<T>)> {
        tokio::task::spawn_blocking(move || {
            let store = self.store.clone();
            let mut db = store.connection().map_err(E::from)?;
            let result = guarded_transaction::run_mut(&mut db, |db| {
                if !db.is_autocommit() {
                    return (Err(Error::Unsettled), true);
                }
                let result = operation(db, &mut self);
                let unknown = !db.is_autocommit()
                    || matches!(&result,Err(Error::Boundary(error)) if error.needs_recovery());
                (result, unknown)
            });
            Ok((self, result))
        })
        .await
        .map_err(|_| Error::Unsettled)?
    }
    async fn release(self) -> Result<Self> {
        let (state, result) = self
            .read(|db, state| {
                state
                    .resources
                    .execute(db, "lease-release", params![state.trace])?;
                state.lease_release_confirmed = Some(true);
                Ok(())
            })
            .await?;
        result?;
        Ok(state)
    }
}
fn consume(
    db: &Connection,
    state: &State,
    reply: Reply,
) -> Result<(
    Decision,
    crate::ai::structured_resource_validation::DecisionOrigin,
)> {
    state.resources.recheck(db, &state.reads)?;
    checked(reply)
}
type Reply = (
    Decision,
    crate::ai::structured_resource_validation::DecisionOrigin,
);
fn checked((decision, origin): Reply) -> Result<Reply> {
    if let Decision::Rejected { message } = decision {
        return Err(Error::Rejected { message, origin });
    }
    Ok((decision, origin))
}
pub(crate) async fn preview(
    service: &LiteratureTraceService,
    store: AppStore,
    key: &str,
    embedding: EmbeddingAccess,
    draft: ConceptRetrievalPreviewDraft,
    signal: CancellationSignal,
) -> std::result::Result<ConceptRetrievalPreview, Value> {
    let client = draft.client_request_id.clone();
    let mut progress = Progress::default();
    let result = run(service, store, key, embedding, draft, signal, &mut progress).await;
    result.map_err(|error| {
        let unknown = matches!(error, Error::Unsettled);
        let mut wire = error.into_wire(
            Some(&client),
            progress.trace.as_deref(),
            "execution",
            HbsOutcome::NotStarted,
        );
        if !unknown {
            if let Some(confirmed) = progress.lease_release_confirmed {
                wire["leaseReleaseConfirmed"] = json!(confirmed);
            }
            wire["traceWriteStarted"] = json!(progress.trace_write_started);
        }
        wire
    })
}
async fn run(
    service: &LiteratureTraceService,
    store: AppStore,
    key: &str,
    embedding: EmbeddingAccess,
    draft: ConceptRetrievalPreviewDraft,
    signal: CancellationSignal,
    progress: &mut Progress,
) -> Result<ConceptRetrievalPreview> {
    let paper = draft.paper_id.clone();
    let (query, origin) = checked(
        service
            .concept_retrieval_rule(key, Input::ValidateDraft { draft })
            .await?,
    )?;
    let (query, fts, query_sha256) = match query {
        Decision::Query {
            query,
            fts_query,
            query_sha256,
        } => (query, fts_query, query_sha256),
        _ => return Err(mismatch(origin)),
    };
    let trace = Uuid::new_v4().to_string();
    progress.trace = Some(trace.clone());
    let resources = service.concept_retrieval_resources(key).await?;
    let mut state = State {
        store,
        resources,
        reads: ReadSet::default(),
        trace,
        paper,
        query,
        fts,
        query_sha256,
        lease_release_confirmed: None,
        trace_write_started: false,
    };
    let paper_reply = if let Some(paper) = state.paper.clone() {
        let (next, result) = state
            .read(move |db, state| {
                capture::paper(db, &state.resources, &mut state.reads, &paper).map_err(Error::Data)
            })
            .await?;
        let exists = result?;
        state = next;
        Some(
            service
                .concept_retrieval_rule(key, Input::ValidatePaper { exists })
                .await?,
        )
    } else {
        None
    };
    let (next, result) = state
        .read(move |db, state| {
            if let Some(reply) = paper_reply {
                let (decision, origin) = consume(db, state, reply)?;
                if !matches!(decision, Decision::PaperValidated) {
                    return Err(mismatch(origin));
                }
            }
            let prepared = embedding.prepare_in(db)?;
            let lexical_ids = capture::lexical(db, &state.resources, &mut state.reads, &state.fts)?;
            let target = capture::vector_target(
                db,
                &state.resources,
                &mut state.reads,
                prepared.model_id(),
            )?;
            if let Some(target) = &target {
                state.resources.execute(
                    db,
                    "lease-insert",
                    params![
                        format!("retrieval-{}", state.trace),
                        target.index_generation_id,
                        state.trace
                    ],
                )?;
                state.lease_release_confirmed = Some(false);
            }
            // These two original preparation results stay frozen across provider IO.
            // The later assembly guard has its own complete, live read prefix.
            state.reads = ReadSet::default();
            Ok((prepared, lexical_ids, target))
        })
        .await?;
    progress.lease_release_confirmed = next.lease_release_confirmed;
    let (prepared, lexical_ids, target) = result?;
    state = next;
    if signal.is_cancelled() {
        state.release().await?;
        progress.lease_release_confirmed = Some(true);
        return Err(E::Canceled.into());
    }
    let Some(target) = target else {
        state.release().await?;
        progress.lease_release_confirmed = Some(true);
        return Err(E::SemanticIndex(SemanticIndexError::EngineOperation {
            code: "active_generation_missing".into(),
        })
        .into());
    };
    let hits = match search(&state.store, prepared, &state.query, &target, &signal).await {
        Ok(hits) => hits,
        Err(error) => {
            state.release().await?;
            progress.lease_release_confirmed = Some(true);
            return Err(error);
        }
    };
    if signal.is_cancelled() {
        state.release().await?;
        progress.lease_release_confirmed = Some(true);
        return Err(E::Canceled.into());
    }
    let selected = target.clone();
    let (next, result) = state
        .read(move |db, state| {
            state
                .resources
                .execute(db, "lease-release", params![state.trace])?;
            state.lease_release_confirmed = Some(true);
            capture::vector_hits(db, &state.resources, &mut state.reads, &selected, hits)
                .map_err(Error::Data)
        })
        .await?;
    progress.lease_release_confirmed = next.lease_release_confirmed;
    let vectors = result?;
    let reply = service
        .concept_retrieval_rule(
            key,
            Input::RankCandidates {
                lexical_ids,
                vector_hits: vectors,
            },
        )
        .await?;
    let (next, result) = next
        .read(move |db, state| {
            let (decision, origin) = consume(db, state, reply)?;
            let Decision::Candidates { candidates } = decision else {
                return Err(mismatch(origin));
            };
            capture::identities(db, &state.resources, &mut state.reads, candidates)
                .map_err(Error::Data)
        })
        .await?;
    let candidates = result?;
    let reply = service
        .concept_retrieval_rule(key, Input::SelectSeeds { candidates })
        .await?;
    let (next, result) = next
        .read(move |db, state| {
            let (decision, origin) = consume(db, state, reply)?;
            let Decision::Seeds { seeds } = decision else {
                return Err(mismatch(origin));
            };
            let seed_ids = seeds
                .iter()
                .map(|seed| seed.concept_id.clone())
                .collect::<Vec<_>>();
            let edges = capture::graph(db, &state.resources, &mut state.reads, &seed_ids)?;
            Ok((seeds, seed_ids, edges))
        })
        .await?;
    let (seeds, seed_ids, edges) = result?;
    let reply = service
        .concept_retrieval_rule(key, Input::ProjectGraph { seed_ids, edges })
        .await?;
    let (next, result) = next
        .read(move |db, state| {
            let (decision, origin) = consume(db, state, reply)?;
            let Decision::Graph { edges, concept_ids } = decision else {
                return Err(mismatch(origin));
            };
            let (definitions, usages) =
                capture::sources(db, &state.resources, &mut state.reads, &concept_ids)?;
            Ok((edges, definitions, usages))
        })
        .await?;
    let (edges, definitions, usages) = result?;
    let input = Input::ProjectSources {
        paper_id: next.paper.clone(),
        seeds: seeds.clone(),
        edges: edges.clone(),
        definitions,
        usages,
    };
    let reply = service.concept_retrieval_rule(key, input).await?;
    let (next, result) = next
        .read(move |db, state| {
            let (decision, origin) = consume(db, state, reply)?;
            let Decision::Trace {
                sources,
                selected_source_count,
                external_paper_count,
                context_text,
                context_sha256,
                config_json,
            } = decision
            else {
                return Err(mismatch(origin));
            };
            persist::publish(
                db,
                &state.resources,
                TraceWrite {
                    trace_id: state.trace.clone(),
                    paper_id: state.paper.clone(),
                    query_sha256: state.query_sha256.clone(),
                    target,
                    seeds,
                    edges,
                    sources,
                    context_text,
                    context_sha256,
                    config_json,
                    selected_source_count,
                    external_paper_count,
                },
                &mut state.trace_write_started,
            )
        })
        .await?;
    progress.trace_write_started = next.trace_write_started;
    result
}
async fn search(
    store: &AppStore,
    prepared: PreparedEmbedding,
    query: &str,
    target: &VectorTargetV1,
    signal: &CancellationSignal,
) -> Result<Vec<SemanticVectorSearchHit>> {
    if prepared.model_id() != target.model_id {
        return Err(E::Provider.into());
    }
    let response = prepared
        .embed(vec![query.into()], target.dimension, signal)
        .await?;
    if response.model_id != target.model_id {
        return Err(E::Provider.into());
    }
    let vector = response.vectors.into_iter().next().ok_or(E::Provider)?;
    let vectors = VectorAccess::new(store.semantic_index_directory().to_path_buf())
        .open(
            &target.relative_database_path,
            SemanticNamespace::AcceptedConcepts,
            target.dimension,
        )
        .await
        .map_err(SemanticIndexError::from)
        .map_err(E::from)?;
    vectors
        .search(&vector, 24)
        .await
        .map_err(SemanticIndexError::from)
        .map_err(E::from)
        .map_err(Into::into)
}
fn mismatch(origin: crate::ai::structured_resource_validation::DecisionOrigin) -> Error {
    let mut error = crate::runtime_plugins::native::NativeHostError::for_profile(
        crate::runtime_plugins::native::NativeProfile::NoteTraceDecision,
        "plugin_host_response_mismatch",
        "检索准备结果阶段与原输入不匹配。",
    );
    error.package_key = Some(origin.package_key);
    error.instance_id = Some(origin.instance_id);
    error.request_id = Some(origin.native_request_id);
    Error::Native(error)
}
