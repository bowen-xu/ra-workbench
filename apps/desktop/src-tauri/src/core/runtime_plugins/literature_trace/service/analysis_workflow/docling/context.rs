//! Original snapshot and screenshot reads stay with Core, before Native image validation.
use super::*;
use crate::{
    ai::{
        paper_reading_contracts::{PaperAiReadingPrepareResult, PaperContextSnapshot, Preparation},
        reading_contracts::StoredImageAsset,
        reading_support::PaperReadingImages,
    },
    runtime_plugins::{data::load_text, literature_context::PaperContextResources},
    storage::AppStore,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::collections::BTreeMap;

type SnapshotInput = (PaperContextSnapshot, Vec<(String, String)>);

impl LiteratureTraceService {
    pub(crate) async fn load_docling_snapshot(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        paper: String,
        snapshot: String,
    ) -> Result<
        (
            PaperAiReadingPrepareResult,
            Preparation,
            BTreeMap<String, StoredImageAsset>,
        ),
        AiError,
    > {
        let owner = Arc::clone(self);
        tokio::spawn(async move { owner.load_snapshot_owned(store, key, paper, snapshot).await })
            .await
            .map_err(|_| AiError::TemporaryUnavailable)?
    }

    async fn load_snapshot_owned(
        &self,
        store: AppStore,
        key: String,
        paper: String,
        snapshot: String,
    ) -> Result<
        (
            PaperAiReadingPrepareResult,
            Preparation,
            BTreeMap<String, StoredImageAsset>,
        ),
        AiError,
    > {
        self.available()
            .map_err(super::super::template::technical)?;
        let _lifetime = self.doi_operations.read().await;
        self.available()
            .map_err(super::super::template::technical)?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.clone();
        let resources = tokio::task::spawn_blocking(move || {
            let package = package::accept_analysis_workflow(&catalog, &selected)?;
            package::accept_paper_context(&catalog, &selected)?;
            let context = PaperContextResources::load(&catalog, &package)
                .map_err(|error| Self::error(error.code, error.message).package(&selected))?;
            let images = load_text(
                &catalog,
                &package,
                "literature_library",
                "data/analysis-docling-snapshot-images.v1.sql",
                "bc265181ce02fca0b7f4916efd29acf905145db06cab8fc6cc1bb3c6869c1a2a",
            )
            .map_err(|error| Self::error(error.code, error.message).package(&selected))?;
            Ok::<_, Box<NativeHostError>>((context, images))
        })
        .await
        .map_err(|_| AiError::TemporaryUnavailable)?
        .map_err(super::super::template::technical)?;
        let source = store.clone();
        let selected_paper = paper.clone();
        let id = snapshot.clone();
        let (resources, captured, assets) = tokio::task::spawn_blocking(move || {
            let db = source.connection()?;
            let captured = read_snapshot(&db, &resources.0, &resources.1, &selected_paper, &id)?;
            let mut assets = BTreeMap::new();
            for (id, hash) in &captured.1 {
                let image = PaperReadingImages::new(&db, &source)
                    .read(hash)
                    .map_err(|error| error.into_ai_error())?;
                assets.insert(
                    id.clone(),
                    StoredImageAsset {
                        context_item_id: id.clone(),
                        content_sha256: hash.clone(),
                        data_url: format!("data:image/png;base64,{}", STANDARD.encode(image.bytes)),
                    },
                );
            }
            Ok::<_, AiError>((resources, captured, assets))
        })
        .await
        .map_err(|_| AiError::TemporaryUnavailable)??;
        let prepared = captured.0.clone().into_reading_result();
        let (decision, origin) = self
            .analysis_workflow_decision(
                &key,
                WorkflowInputV1::ValidateDoclingImages {
                    requested_ids: prepared
                        .image_requests
                        .iter()
                        .map(|image| image.context_item_id.clone())
                        .collect(),
                    available_ids: assets.keys().cloned().collect(),
                },
            )
            .await
            .map_err(super::super::template::technical)?;
        match decision {
            WorkflowDecisionV1::Validated { rejection: None } => {}
            WorkflowDecisionV1::Validated {
                rejection: Some(error),
            } => return Err(super::super::template::rejected(error, &origin)),
            _ => {
                return Err(super::super::template::identified(
                    "plugin_host_response_mismatch",
                    "Docling 图片校验结果种类不匹配。",
                    &origin,
                ));
            }
        }
        let expected = captured.clone();
        let unchanged = tokio::task::spawn_blocking(move || {
            let db = store.connection()?;
            read_snapshot(&db, &resources.0, &resources.1, &paper, &snapshot)
                .map(|value| value == expected)
        })
        .await
        .map_err(|_| AiError::TemporaryUnavailable)??;
        if !unchanged {
            return Err(super::super::template::identified(
                "plugin_host_source_changed",
                "Docling 快照在准备期间已变化，请重新载入。",
                &origin,
            ));
        }
        Ok((
            prepared,
            Preparation {
                schema_version: 1,
                paper_id: captured.0.paper_id,
                snapshot_id: captured.0.id,
                context_hash: captured.0.context_hash,
            },
            assets,
        ))
    }
}

fn read_snapshot(
    db: &rusqlite::Connection,
    context: &PaperContextResources,
    sql: &str,
    paper: &str,
    id: &str,
) -> Result<SnapshotInput, AiError> {
    let snapshot = context.load_snapshot(db, id).map_err(|failure| {
        // This is an original read. Preserve the fixed package resource cause when available.
        if let crate::ai::structured_resource_validation::FailureCause::SourceData(error) =
            failure.failure.cause
        {
            AiError::PluginDatabase(error)
        } else {
            failure.failure.error
        }
    })?;
    if snapshot.paper_id != paper {
        return Err(AiError::InvalidInput("论文上下文快照不属于当前文献".into()));
    }
    let mut statement = db.prepare(sql)?;
    let rows = statement.query_map([id], |row| Ok((row.get(0)?, row.get(1)?)))?;
    let images = rows.collect::<Result<Vec<_>, _>>()?;
    Ok((snapshot, images))
}
