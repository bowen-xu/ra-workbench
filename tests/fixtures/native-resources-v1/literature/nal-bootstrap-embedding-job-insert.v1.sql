insert into embedding_outbox_jobs
         (id, embedding_record_id, attempt, status, idempotency_key, created_at)
         values (?1, ?2, 1, 'queued', ?3, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))