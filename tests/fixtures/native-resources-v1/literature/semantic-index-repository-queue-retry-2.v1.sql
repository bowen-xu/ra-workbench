select coalesce(max(attempt), 0) + 1 from embedding_outbox_jobs
         where embedding_record_id = ?1