update embedding_reconciliation_runs
         set status = ?2, checked_count = ?3, missing_count = ?4,
             orphan_count = ?5, repaired_count = ?6, error_code = ?7,
             finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and status = 'running'