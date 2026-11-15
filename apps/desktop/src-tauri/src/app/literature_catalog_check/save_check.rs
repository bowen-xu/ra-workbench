//! Real independent-host manual saves against the same synthetic Core database.
use super::*;
use crate::runtime_plugins::literature_catalog::save::SaveOutcome;
use research_workbench_native_sdk::catalog_save::CatalogDraft;

fn draft(id: Option<&str>, title: &str) -> CatalogDraft {
    serde_json::from_value(json!({"id":id,"itemType":"journalArticle","title":title,
        "issuedYear":2026,"rawBibtex":"@article{native-save}","sourceKind":null,
        "doi":"DOI: 10.1000/Native", "abstractText":"  Saved abstract  ",
        "creators":[{"familyName":" Family ","givenName":" Given ","role":"author"}],
        "tags":[" Tagged "," "]}))
    .expect("fixed synthetic draft")
}

pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let create = draft(None, "  Native saved paper  ");
    let initial = service
        .save_catalog(store.shared_agent_connection(), key.into(), create)
        .await?;
    if initial.title != "Native saved paper"
        || initial.doi != "10.1000/Native"
        || initial.tags != ["Tagged"]
        || initial.metadata_sources.len() != 17
        || initial
            .metadata_sources
            .iter()
            .any(|source| !source.confirmed_by_user)
        || initial.creators[0].family_name != "Family"
    {
        return Err("native manual save lost original normalization/provenance".into());
    }
    let id = initial.id.clone();
    uuid::Uuid::parse_str(&id)?;
    let mut update = draft(Some(&id), "  Updated native paper  ");
    update.raw_bibtex = None;
    let updated = service
        .save_catalog(store.shared_agent_connection(), key.into(), update.clone())
        .await?;
    if updated.created_at != initial.created_at || updated.raw_bibtex != initial.raw_bibtex {
        return Err("native update lost creation time or existing raw BibTeX".into());
    }
    let search = service
        .catalog(
            store.shared_agent_connection(),
            key.into(),
            CatalogRequest::List {
                query: Some("UPDATED".into()),
            },
        )
        .await?;
    if search
        .iter()
        .map(|item| item.id.as_str())
        .collect::<Vec<_>>()
        != [id.as_str()]
    {
        return Err("native saved FTS did not reflect the update".into());
    }
    update.title = "This must roll back".into();
    update.tags = vec!["Duplicate tag".into(), "duplicate tag".into()];
    let error = service
        .save_catalog(store.shared_agent_connection(), key.into(), update)
        .await
        .unwrap_err();
    if error.code != "storage_error" || !matches!(error.save_outcome, Some(SaveOutcome::RolledBack))
    {
        return Err(format!("native duplicate tag did not roll back: {error:?}").into());
    }
    let reloaded = service
        .catalog(
            store.shared_agent_connection(),
            key.into(),
            CatalogRequest::Get { id: id.clone() },
        )
        .await?;
    if reloaded != [updated.clone()] {
        return Err("rolled-back save changed the original item".into());
    }

    let mut invalid = draft(None, "");
    invalid.item_type = "unsupported".into();
    let error = service
        .save_catalog(store.shared_agent_connection(), key.into(), invalid)
        .await
        .unwrap_err();
    if error.code != "invalid_input"
        || error.message != "不支持的文献类型：unsupported"
        || !matches!(error.save_outcome, Some(SaveOutcome::NotStarted))
    {
        return Err("native validation changed original type/title priority".into());
    }
    let error = service
        .save_catalog(
            store.shared_agent_connection(),
            key.into(),
            draft(None, " \t "),
        )
        .await
        .unwrap_err();
    if error.code != "invalid_input"
        || error.message != "标题不能为空"
        || !matches!(error.save_outcome, Some(SaveOutcome::NotStarted))
    {
        return Err("native save changed original empty-title diagnostic".into());
    }
    let hidden = draft(Some("paper-trashed"), "Hidden save committed");
    let error = service
        .save_catalog(store.shared_agent_connection(), key.into(), hidden)
        .await
        .unwrap_err();
    if error.code != "not_found"
        || !matches!(error.save_outcome,Some(SaveOutcome::Committed {ref item_id}) if item_id=="paper-trashed")
    {
        return Err(format!("hidden save lost committed outcome: {error:?}").into());
    }
    let hidden_title: String = store.connection()?.query_row(
        "select title from bibliographic_items where id='paper-trashed' and trashed_at is not null",
        [],
        |row| row.get(0),
    )?;
    if hidden_title != "Hidden save committed" {
        return Err("hidden save was rejected before its original commit".into());
    }
    Ok(
        json!({"status":"passed-native-save-only","item":updated,"createdId":id,
        "duplicateTagRollback":true,"validationPriority":true,"hiddenCommittedNotFound":true}),
    )
}

pub(super) async fn reopen(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    directory: &Path,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let original: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("receipt.json"))?)?;
    let expected: CatalogItem = serde_json::from_value(original["item"].clone())?;
    let items = service
        .catalog(
            store.shared_agent_connection(),
            key.into(),
            CatalogRequest::Get {
                id: expected.id.clone(),
            },
        )
        .await?;
    if items != [expected] {
        return Err("cold-reopened Core catalog differs from the committed item".into());
    }
    let hidden: String = store.connection()?.query_row(
        "select title from bibliographic_items where id='paper-trashed' and trashed_at is not null",
        [],
        |row| row.get(0),
    )?;
    if hidden != "Hidden save committed" {
        return Err("hidden committed save was lost on reopen".into());
    }
    Ok(
        json!({"status":"passed-cold-reopen","originalReceipt":"receipt.json","sameItem":true,"hiddenCommitted":true}),
    )
}
