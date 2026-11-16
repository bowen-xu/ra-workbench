select id, status, checked_count, missing_count, orphan_count, repaired_count,
                    error_code, started_at, finished_at
             from embedding_reconciliation_runs where id = ?1