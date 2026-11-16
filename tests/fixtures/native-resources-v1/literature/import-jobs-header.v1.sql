select id, state, source_kind, total_count, completed_count,
                    cancellation_requested, created_at, updated_at
             from import_jobs where id = ?1