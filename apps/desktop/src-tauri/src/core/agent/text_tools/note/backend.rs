//! Storage-port implementation; preparation and mapping remain in the note adapter.
use super::*;
impl TextBackend for NoteBackend {
    fn read<'a>(
        &'a self,
        path: &'a str,
        saved: bool,
        remember: bool,
    ) -> BackendFuture<'a, TextSnapshot> {
        Box::pin(async move {
            let db = self.store.connection().map_err(|error| error.to_string())?;
            self.model_eligible(&db)?;
            let snapshot = self.snapshot(&db, path, saved)?;
            if remember {
                journal::remember(&db, &self.record, &snapshot)?;
            }
            Ok(snapshot)
        })
    }
    fn preflight<'a>(&'a self, changes: &'a [TextChange]) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            let db = self.store.connection().map_err(|error| error.to_string())?;
            self.prepare(&db, changes, "preflight")?;
            Ok(())
        })
    }
    fn commit<'a>(
        &'a self,
        request_id: &'a str,
        changes: Vec<TextChange>,
    ) -> BackendFuture<'a, Vec<TargetResult>> {
        Box::pin(async move {
            let input = serde_json::to_string(&changes).map_err(|error| error.to_string())?;
            let (mut group, previous_generation, previous_signature, previous_notes) = {
                let mut db = match self.store.connection() {
                    Ok(db) => db,
                    Err(error) => {
                        return Ok(patch_journal::unknown_changes(
                            &changes,
                            &patch_journal::query_owner_diagnostic(
                                &self.record,
                                request_id,
                                &error,
                            ),
                        ));
                    }
                };
                if !db.is_autocommit() {
                    db.require_recovery();
                }
                if let Some(result) =
                    receipt::lookup(&db, &self.record, request_id, &input, &changes)?
                {
                    return Ok(result);
                }
                let group = self.prepare(&db, &changes, request_id)?;
                let generation:i64=db.query_row("select coalesce(max(generation),0) from agent_note_patch_candidates where conversation_id=?1",[self.record.conversation_id.to_string()],|row|row.get(0)).map_err(|error|error.to_string())?;
                (
                    group,
                    generation,
                    signature(&db, self.record.conversation_id)?,
                    admission::pending_changes(self, &db, &changes)?,
                )
            };
            resources::import(self, &mut group)?;
            let mut db = match self.store.connection() {
                Ok(db) => db,
                Err(error) => {
                    return Ok(patch_journal::unknown_changes(
                        &changes,
                        &patch_journal::query_owner_diagnostic(&self.record, request_id, &error),
                    ));
                }
            };
            if !db.is_autocommit() {
                db.require_recovery();
            }
            if let Some(result) = receipt::lookup(&db, &self.record, request_id, &input, &changes)?
            {
                return Ok(result);
            }
            let mut tx =
                rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
                    .map_err(|error| {
                        patch_journal::prewrite_diagnostic(
                            self.record.conversation_id,
                            request_id,
                            "note-begin",
                            &error,
                        )
                        .to_string()
                    })?;
            // Check every explicit cleanup; Drop must not hide a failed rollback.
            tx.set_drop_behavior(rusqlite::DropBehavior::Ignore);
            let prepared = (|| -> Result<Vec<TargetResult>, String> {
                self.model_eligible(&tx)?;
                let generation_now:i64=tx.query_row("select coalesce(max(generation),0) from agent_note_patch_candidates where conversation_id=?1",[self.record.conversation_id.to_string()],|row|row.get(0)).map_err(|error|error.to_string())?;
                if generation_now != previous_generation
                    || signature(&tx, self.record.conversation_id)? != previous_signature
                {
                    return Err("candidate changed during asset preparation".into());
                }
                for note in &group {
                    resources::check_imports(self, &tx, note)?;
                    if let Some(parent) = note
                        .candidate
                        .as_ref()
                        .and_then(|page| page.parent_id.as_deref())
                    {
                        self.authorize(&tx, parent, true)?;
                    }
                    if let Some(base) = note.saved.as_ref().or(note.baseline.as_ref()) {
                        self.authorize(&tx, &base.page_id, true)?;
                        if serde_json::to_value(load(&tx, &base.page_id)?)
                            .map_err(|error| error.to_string())?
                            != serde_json::to_value(base).map_err(|error| error.to_string())?
                        {
                            return Err("saved note changed during asset preparation".into());
                        }
                    }
                }
                journal::insert_sql(&tx, self.record.conversation_id, request_id, &input)
                    .map_err(|error| error.target_message(&self.record, request_id))?;
                let generation:i64=tx.query_row("select coalesce(max(generation),0)+1 from agent_note_patch_candidates where conversation_id=?1",[self.record.conversation_id.to_string()],|row|row.get(0)).map_err(|error|error.to_string())?;
                let proposal = format!(
                    "note-patch:{:x}",
                    Sha256::digest(format!(
                        "{}:{request_id}:{generation}",
                        self.record.conversation_id
                    ))
                );
                let mut turn_snapshot =
                    super::super::note_review::snapshots::for_turn(&group, &previous_notes);
                // Persist current D and B before acknowledging delivery. Pending
                // review is independent from the saved note, as in writing files.
                super::super::note_review::persistence::checkpoint(&tx, &mut group)?;
                let delivery = delivery::freeze(&mut turn_snapshot, &group, &proposal, generation);
                let retained = group
                    .iter()
                    .filter_map(|note| note.candidate.as_ref().or(note.baseline.as_ref()))
                    .map(|page| page.page_id.clone())
                    .collect::<std::collections::BTreeSet<_>>();
                // Only explicitly addressed prior groups can be cancelled.
                // Other proposals retain their content, assets and frozen history.
                for old in &previous_notes {
                    let page = old
                        .candidate
                        .as_ref()
                        .or(old.baseline.as_ref())
                        .ok_or("missing prior identity")?;
                    if !retained.contains(&page.page_id) {
                        tx.execute("update agent_note_patch_candidates set state='rejected' where conversation_id=?1 and page_id=?2",params![self.record.conversation_id.to_string(),page.page_id]).map_err(|error|error.to_string())?;
                        tx.execute("delete from agent_note_patch_assets where conversation_id=?1 and page_id=?2",params![self.record.conversation_id.to_string(),page.page_id]).map_err(|error|error.to_string())?;
                    }
                }
                for note in &group {
                    let page = note
                        .candidate
                        .as_ref()
                        .or(note.baseline.as_ref())
                        .ok_or("missing note identity")?;
                    tx.execute("insert into agent_note_patch_candidates(conversation_id,page_id,proposal_id,generation,baseline_json,candidate_json,state,persistence_version) values(?1,?2,?3,?4,?5,?6,'pending',2) on conflict(conversation_id,page_id) do update set proposal_id=excluded.proposal_id,generation=excluded.generation,baseline_json=excluded.baseline_json,candidate_json=excluded.candidate_json,state='pending',persistence_version=2",params![self.record.conversation_id.to_string(),page.page_id,proposal,generation,serde_json::to_string(&note.baseline).map_err(|error|error.to_string())?,serde_json::to_string(note).map_err(|error|error.to_string())?]).map_err(|error|error.to_string())?;
                    resources::retain_assets(&tx, self.record.conversation_id, note)?;
                }
                super::super::note_review::snapshots::record(
                    &tx,
                    self.record.conversation_id,
                    &proposal,
                    generation,
                    &turn_snapshot,
                )?;
                let results=changes.iter().map(|change|{
            let path=group.iter().find(|note|note.original_path==change.path||note.path==change.path||change.destination.as_ref()==Some(&note.path)).map(|note|note.path.as_str()).unwrap_or(&change.path);
            TargetResult{path:change.path.clone(),status:TargetStatus::Applied,message:format!("Note saved with pending review; Keep/Undo remains available. Continue reading at {path}."),change_id:Some(proposal.clone()),generation:Some(generation),name:group.iter().find(|note|note.path==path).and_then(|note|note.candidate.as_ref().or(note.baseline.as_ref())).map(|page|page.title.clone())}
        }).collect::<Vec<_>>();
                journal::record_sql(
                    &tx,
                    self.record.conversation_id,
                    request_id,
                    &json!({"targets":results,"noteDelivery":delivery}),
                    true,
                )
                .map_err(|error| {
                    patch_journal::target_diagnostic(
                        &self.record,
                        request_id,
                        "target-terminal",
                        &error,
                    )
                    .to_string()
                })?;
                Ok(results)
            })();
            match prepared {
                Ok(results) => match tx.commit() {
                    Ok(()) if db.is_autocommit() => Ok(results),
                    outcome => {
                        let detail = receipt::transaction_detail(
                            &self.record,
                            request_id,
                            "note-commit",
                            outcome.as_ref().err(),
                        );
                        // Keep the commit's original driver evidence. Cleanup is
                        // explicit, and no new admission may use an uncertain base.
                        if !db.is_autocommit() && db.execute_batch("rollback;").is_err() {
                            patch_journal::report(&receipt::transaction_detail(
                                &self.record,
                                request_id,
                                "note-rollback",
                                None,
                            ));
                        }
                        db.require_recovery();
                        patch_journal::report(&detail);
                        Ok(patch_journal::unknown_changes(&changes, &detail))
                    }
                },
                Err(message) => match tx.rollback() {
                    Ok(()) if db.is_autocommit() => Err(message),
                    outcome => {
                        let detail = receipt::transaction_detail(
                            &self.record,
                            request_id,
                            "note-rollback",
                            outcome.as_ref().err(),
                        );
                        db.require_recovery();
                        patch_journal::report(&detail);
                        Ok(patch_journal::unknown_changes(&changes, &detail))
                    }
                },
            }
        })
    }
    fn query<'a>(&'a self, request_id: &'a str) -> BackendFuture<'a, Option<Vec<TargetResult>>> {
        Box::pin(async move {
            let db = self.store.connection().map_err(|error| {
                patch_journal::query_owner_diagnostic(&self.record, request_id, &error).to_string()
            })?;
            receipt::query(&db, &self.record, request_id)
        })
    }
}
fn signature(db: &Connection, conversation: Uuid) -> Result<String, String> {
    let mut query = db.prepare("select page_id,proposal_id,generation,candidate_json from agent_note_patch_candidates where conversation_id=?1 and state='pending' order by page_id").map_err(|error|error.to_string())?;
    let rows = query
        .query_map([conversation.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&rows).map_err(|error| error.to_string())?)
    ))
}
