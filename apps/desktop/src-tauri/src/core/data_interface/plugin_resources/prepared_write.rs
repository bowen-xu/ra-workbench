//! Borrowed fixed writes for an existing caller's prepare-all execution boundary.
use super::*;

/// The accepted resource and original arguments cannot be replaced after
/// preparation. This handle borrows its caller's transaction and never crosses
/// an await, owns admission, or starts/settles a transaction.
pub(crate) struct PreparedDataWrite<'connection, 'resource> {
    connection: &'connection Connection,
    statement: Statement<'connection>,
    resource: &'resource AcceptedDataResource,
    call: DataCall,
}

impl<'connection, 'resource> PreparedDataWrite<'connection, 'resource> {
    pub(crate) fn prepare_in(
        tx: &'connection Transaction<'_>,
        resource: &'resource AcceptedDataResource,
        call: DataCall,
    ) -> Result<Self> {
        let statement = prepare_statement(
            tx,
            resource,
            &call,
            DataAccess::Write,
            TransactionRequirement::Required,
        )?;
        Ok(Self {
            connection: tx,
            statement,
            resource,
            call,
        })
    }

    pub(crate) fn execute(&mut self) -> Result<u64> {
        // A raw compatibility caller can explicitly end its transaction even
        // while a statement borrows it. Never turn that into an autocommit write.
        if self.connection.is_autocommit() {
            let mut error = failure(
                self.resource,
                "validate",
                "plugin_data_call_invalid",
                "数据操作的名称、版本、参数或读写种类与已授予契约不符。",
            );
            error.safe_cause = "caller_transaction_missing".into();
            return Err(error);
        }
        let result = execute_write(&mut self.statement, self.resource, &self.call)?;
        validate_result(self.resource, &result)?;
        match result {
            DataResult::Affected(count) => Ok(count),
            DataResult::Rows(_) => unreachable!("fixed write only returns affected rows"),
        }
    }
}
