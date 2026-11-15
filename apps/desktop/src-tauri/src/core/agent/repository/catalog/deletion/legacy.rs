//! SQL-only historical fixtures. Production never falls back to this path.
use super::*;
fn replay(db: &Connection, request: &Request, hash: &str) -> Result<Option<Receipt>, CatalogError> {
    replay_sql(db, request, hash).map_err(|e| match e {
        DeletionSqlError::Domain(Error::Catalog(e)) => e,
        _ => CatalogError::Unavailable,
    })
}
impl SqliteConversationCatalog {
    pub(crate) fn delete_legacy_for_tests(
        &self,
        request: Request,
    ) -> ConversationDeletionFuture<'_> {
        Box::pin(async move {
            if self.connection.logged() {
                return Err(CatalogError::InvalidInput.into());
            }
            request.validate()?;
            let hash = request_hash(&request);
            let replay_request = request.clone();
            let replay_hash = hash.clone();
            if let Some(receipt) = self
                .run(move |db| replay(db, &replay_request, &replay_hash))
                .await?
            {
                return Ok(receipt);
            }
            let preparation = async {
                let summary = self
                    .read(request.conversation_id, request.binding.clone())
                    .await?;
                check_summary(&summary, &request)?;
                let snapshot = self
                    .export_snapshot(request.conversation_id)
                    .await
                    .map_err(|error| match error {
                        SnapshotExportError::NotFound => CatalogError::NotFound,
                        SnapshotExportError::TooLarge => CatalogError::TooLarge,
                        SnapshotExportError::Corrupt => CatalogError::Corrupt,
                        SnapshotExportError::Unavailable => CatalogError::Unavailable,
                    })?;
                if snapshot.turns.iter().any(|turn| turn.outcome.is_none()) {
                    return Err(CatalogError::Busy.into());
                }
                if snapshot.watermark != request.expected_durable_count {
                    return Err(CatalogError::Conflict.into());
                }
                Ok::<(), Error>(())
            }
            .await;
            if let Err(error) = preparation {
                // An identical concurrent deletion may win while the snapshot is read.
                return match self.run(move |db| replay(db, &request, &hash)).await? {
                    Some(receipt) => Ok(receipt),
                    None => Err(error),
                };
            }
            let workspace_directory = self.workspace_directory.clone();
            self.run(move |db| Ok(delete_legacy(db, &request, &hash, workspace_directory)))
                .await?
        })
    }
}

fn delete_legacy(
    db: &mut Connection,
    request: &Request,
    hash: &str,
    root: Option<std::path::PathBuf>,
) -> Result<Receipt, Error> {
    let rows = workspaces(db, request.conversation_id).map_err(domain_or)?;
    if request.force && root.is_some() {
        purge_workspace_files(&rows, request, root)?;
    }
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_error)?;
    if let Some(saved) = replay_sql(&tx, request, hash).map_err(domain_or)? {
        return Ok(saved);
    }
    let prepared = PreparedDeletion {
        request: request.clone(),
        hash: hash.into(),
        workspaces: rows,
        cleanup_complete: false,
        #[cfg(unix)]
        _workspace_guards: Vec::new(),
    };
    delete_in(&tx, &prepared).map_err(domain_or)?;
    tx.commit().map_err(db_error)?;
    Ok(receipt(request))
}
