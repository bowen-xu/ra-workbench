use super::{DerivedError as SynthesisError, DerivedPolicy, DerivedSql};
use research_workbench_native_sdk::derived_view::{DerivedResearchView, DerivedResearchViewDraft};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

pub(crate) fn save_derived_view_from(
    sql: &DerivedSql,
    policy: &mut dyn DerivedPolicy,
    explicit: bool,
    connection: &mut Connection,
    draft: DerivedResearchViewDraft,
) -> Result<DerivedResearchView, SynthesisError> {
    super::validate_with(
        sql.get("input-snapshot"),
        connection,
        &draft,
        &mut |input| policy.validate(input),
    )?;
    let mut identity = draft.id.clone();
    let transaction =
        Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Deferred)?;
    let result = match draft.id.as_deref() {
        None => insert_view(sql, &transaction, &draft, &mut identity),
        Some(id) => update_view(sql, policy, &transaction, id, &draft),
    };
    let id = super::settle(connection, transaction, result, explicit).map_err(|cause| {
        if matches!(cause, SynthesisError::Need(_)) {
            return cause;
        }
        match (explicit, identity) {
            (true, Some(id)) => SynthesisError::Identified {
                id,
                cause: Box::new(cause),
            },
            _ => cause,
        }
    })?;
    derived_view_from(sql, connection, &id).map_err(|cause| {
        if explicit {
            SynthesisError::Readback {
                id,
                revision: None,
                cause: Box::new(cause),
            }
        } else {
            cause
        }
    })
}

pub(crate) fn list_derived_views_from(
    sql: &DerivedSql,
    connection: &Connection,
    project_id: &str,
) -> Result<Vec<DerivedResearchView>, SynthesisError> {
    let mut statement = connection.prepare(sql.get("ids"))?;
    let ids = statement
        .query_map([project_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    ids.iter()
        .map(|id| derived_view_from(sql, connection, id))
        .collect()
}

fn insert_view(
    sql: &DerivedSql,
    transaction: &Transaction<'_>,
    draft: &DerivedResearchViewDraft,
    identity: &mut Option<String>,
) -> Result<String, SynthesisError> {
    if draft.expected_revision.is_some() {
        return Err(SynthesisError::InvalidInput(
            "新派生视图不能携带预期版本".to_owned(),
        ));
    }
    let id = Uuid::new_v4().to_string();
    *identity = Some(id.clone());
    transaction.execute(
        sql.get("insert"),
        params![
            id,
            draft.project_id,
            draft.view_kind,
            draft.title.trim(),
            draft.scope_note.trim(),
            json(&draft.inputs, "派生视图输入")?,
            json(&draft.items, "派生视图条目")?,
            draft.review_state,
        ],
    )?;
    insert_revision(sql, transaction, &id, 1, draft)?;
    Ok(id)
}

fn update_view(
    sql: &DerivedSql,
    policy: &mut dyn DerivedPolicy,
    transaction: &Transaction<'_>,
    id: &str,
    draft: &DerivedResearchViewDraft,
) -> Result<String, SynthesisError> {
    let (project_id, current_revision, provenance_kind) = transaction
        .query_row(sql.get("header"), [id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .optional()?
        .ok_or(SynthesisError::NotFound)?;
    if project_id != draft.project_id {
        return Err(SynthesisError::InvalidInput(
            "派生视图不能移动到另一项目".to_owned(),
        ));
    }
    if draft.expected_revision != Some(current_revision) {
        return Err(SynthesisError::RevisionConflict { current_revision });
    }
    let next_revision = current_revision + 1;
    let next_provenance = policy.provenance(provenance_kind, draft.review_state.clone())?;
    let changed = transaction.execute(
        sql.get("update"),
        params![
            id,
            draft.view_kind,
            draft.title.trim(),
            draft.scope_note.trim(),
            json(&draft.inputs, "派生视图输入")?,
            json(&draft.items, "派生视图条目")?,
            draft.review_state,
            next_provenance,
            next_revision,
            current_revision,
        ],
    )?;
    if changed != 1 {
        return Err(SynthesisError::RevisionConflict { current_revision });
    }
    insert_revision(sql, transaction, id, next_revision, draft)?;
    Ok(id.to_owned())
}

fn insert_revision(
    sql: &DerivedSql,
    transaction: &Transaction<'_>,
    id: &str,
    revision: i64,
    draft: &DerivedResearchViewDraft,
) -> Result<(), SynthesisError> {
    transaction.execute(
        sql.get("revision"),
        params![
            Uuid::new_v4().to_string(),
            id,
            revision,
            json(draft, "派生视图版本")?,
            draft.change_reason.trim(),
        ],
    )?;
    Ok(())
}

fn derived_view_from(
    sql: &DerivedSql,
    connection: &Connection,
    id: &str,
) -> Result<DerivedResearchView, SynthesisError> {
    let value = connection
        .query_row(sql.get("view"), [id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, i64>(12)?,
                row.get::<_, String>(13)?,
                row.get::<_, String>(14)?,
            ))
        })
        .optional()?
        .ok_or(SynthesisError::NotFound)?;
    Ok(DerivedResearchView {
        id: value.0,
        project_id: value.1,
        view_kind: value.2,
        title: value.3,
        scope_note: value.4,
        inputs: decode_json(&value.5, "派生视图输入")?,
        items: decode_json(&value.6, "派生视图条目")?,
        review_state: value.7,
        provenance_kind: value.8,
        provider_id: value.9,
        model_id: value.10,
        generated_at: value.11,
        revision: value.12,
        created_at: value.13,
        updated_at: value.14,
    })
}

fn json<T: serde::Serialize + ?Sized>(value: &T, label: &str) -> Result<String, SynthesisError> {
    serde_json::to_string(value)
        .map_err(|_| SynthesisError::InvalidInput(format!("{label}无法序列化")))
}

fn decode_json<T: serde::de::DeserializeOwned>(
    value: &str,
    label: &str,
) -> Result<T, SynthesisError> {
    serde_json::from_str(value)
        .map_err(|_| SynthesisError::InvalidInput(format!("{label}存储格式无效")))
}
