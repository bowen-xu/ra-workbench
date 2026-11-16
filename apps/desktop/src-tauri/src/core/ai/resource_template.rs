//! Frozen global-template references without overloading historical portrait IDs.
use super::{
    error::AiError, model::AiOverviewProfileContext, paper_template::PaperTemplateSnapshot,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

fn context(
    paper_id: String,
    selection_revision: i64,
    template: PaperTemplateSnapshot,
) -> Result<AiOverviewProfileContext, AiError> {
    if !super::paper_template::valid_snapshot(&template) {
        return Err(AiError::InvalidInput("阅读模板快照无效".into()));
    }
    Ok(AiOverviewProfileContext {
        paper_id,
        selection_revision,
        profile_refs: vec![],
        field_kinds: template
            .fields
            .iter()
            .map(|field| field.id.clone())
            .collect(),
        paper_template: Some(template),
    })
}

pub(super) fn insert_run(
    tx: &Transaction<'_>,
    run: &str,
    value: &AiOverviewProfileContext,
) -> Result<(), AiError> {
    let template = value
        .paper_template
        .as_ref()
        .ok_or(AiError::TemporaryUnavailable)?;
    tx.execute("insert into ai_resource_template_runs(run_id,paper_id,template_id,template_revision,selection_revision,template_json) values(?1,?2,?3,?4,?5,?6)",params![run,value.paper_id,template.reference.id(),template.reference.revision(),value.selection_revision,serde_json::to_string(template).map_err(|_|AiError::TemporaryUnavailable)?])?;
    Ok(())
}

pub(super) fn load_run(
    db: &Connection,
    run: &str,
) -> Result<Option<AiOverviewProfileContext>, AiError> {
    let exists:bool=db.query_row("select exists(select 1 from sqlite_master where type='table' and name='ai_resource_template_runs')",[],|row|row.get(0))?;
    if !exists {
        return Ok(None);
    }
    let row:Option<(String,i64,String)>=db.query_row("select paper_id,selection_revision,template_json from ai_resource_template_runs where run_id=?1",[run],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    row.map(|(paper, revision, json)| {
        context(
            paper,
            revision,
            serde_json::from_str(&json).map_err(|_| AiError::TemporaryUnavailable)?,
        )
    })
    .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::candidate_contracts::PaperTemplateRef;
    use serde_json::json;

    #[test]
    fn global_template_snapshot_has_its_own_identity_not_a_portrait_candidate() {
        let value = json!({"templateResourceId":"template-object","templateResourceRevision":3,"fields":[{"id":"question","title":"问题","question":"本文回应什么问题？","claimIds":[]}]});
        let snapshot: PaperTemplateSnapshot = serde_json::from_value(value.clone()).unwrap();
        assert!(super::super::paper_template::valid_snapshot(&snapshot));
        assert_eq!(snapshot.reference.id(), "template-object");
        assert_eq!(snapshot.reference.revision(), 3);
        assert!(matches!(
            snapshot.reference,
            PaperTemplateRef::Resource { .. }
        ));
        assert_eq!(serde_json::to_value(snapshot).unwrap(), value);
    }

    #[test]
    fn legacy_snapshot_identity_round_trips_without_rewriting_history() {
        let value = json!({"portraitCandidateId":"old-candidate","portraitRevision":2,"fields":[{"id":"method","title":"方法","question":"采用什么方法？","claimIds":["mode_formal"]}]});
        let snapshot: PaperTemplateSnapshot = serde_json::from_value(value.clone()).unwrap();
        assert!(matches!(
            snapshot.reference,
            PaperTemplateRef::Legacy { .. }
        ));
        assert_eq!(serde_json::to_value(snapshot).unwrap(), value);
    }

    #[test]
    fn snapshots_reject_mixed_partial_and_unknown_identity_fields() {
        let base = json!({"templateResourceId":"template","templateResourceRevision":1,"fields":[{"id":"question","title":"问题","question":"什么问题？","claimIds":[]}]});
        let mut mixed = base.clone();
        mixed["portraitCandidateId"] = json!("portrait");
        mixed["portraitRevision"] = json!(2);
        assert!(serde_json::from_value::<PaperTemplateSnapshot>(mixed).is_err());
        let mut partial = base.clone();
        partial
            .as_object_mut()
            .unwrap()
            .remove("templateResourceRevision");
        assert!(serde_json::from_value::<PaperTemplateSnapshot>(partial).is_err());
        let mut unknown = base;
        unknown["extra"] = json!(true);
        assert!(serde_json::from_value::<PaperTemplateSnapshot>(unknown).is_err());
    }

    #[test]
    fn migrated_store_reports_managed_resources_to_template_policy() {
        let mut db = Connection::open_in_memory().unwrap();
        assert!(
            !super::super::template_facts::TemplateFacts::new(&db)
                .managed_resources()
                .unwrap()
        );
        crate::storage::migrate(&mut db).unwrap();
        assert!(
            super::super::template_facts::TemplateFacts::new(&db)
                .managed_resources()
                .unwrap()
        );
    }

    #[test]
    fn freezes_canonical_template_revision_independently_of_legacy_snapshot_table() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("create table ai_resource_template_runs(run_id text primary key,paper_id text,template_id text,template_revision integer,selection_revision integer,template_json text);").unwrap();
        let snapshot:PaperTemplateSnapshot=serde_json::from_value(json!({"templateResourceId":"template","templateResourceRevision":4,"fields":[{"id":"result","title":"结果","question":"得到什么结果？","claimIds":[]}]})).unwrap();
        let frozen = context("paper".into(), 7, snapshot).unwrap();
        let tx = db.transaction().unwrap();
        insert_run(&tx, "run", &frozen).unwrap();
        tx.commit().unwrap();
        let restored = load_run(&db, "run").unwrap().unwrap();
        assert_eq!(restored.selection_revision, 7);
        assert_eq!(restored.paper_template.unwrap().reference.revision(), 4);
        assert!(load_run(&db, "another-run").unwrap().is_none());
    }
}
