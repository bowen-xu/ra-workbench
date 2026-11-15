//! Explicit SQL-only historical fixtures; production never writes through this path.
use super::*;
impl SqliteConversationCatalog {
    pub(crate) fn fork_legacy_for_tests(
        &self,
        request: ConversationForkRequest,
    ) -> CatalogFuture<'_, ConversationFork> {
        Box::pin(async move {
            if self.connection.logged() {
                return Err(CatalogError::InvalidInput);
            }
            validate_fork_request(&request)?;
            let probe = request.clone();
            if let Some(saved) = self.run(move |db| replay(db, &probe)).await? {
                return Ok(saved);
            }
            let prepared = self.prepare_fork(request.clone()).await?;
            self.run(move |db| {
                let tx = db
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(db_error)?;
                if let Some(saved) = replay(&tx, &request)? {
                    return Ok(saved);
                }
                fork_in(&tx, &prepared).map_err(|error| match error {
                    crate::data_interface::SqlMutationError::Domain(error) => error,
                    crate::data_interface::SqlMutationError::Sql(error) => db_error(error),
                })?;
                let result = replay(&tx, &request)?.ok_or(CatalogError::Corrupt)?;
                tx.commit().map_err(db_error)?;
                Ok(result)
            })
            .await
        })
    }
}
