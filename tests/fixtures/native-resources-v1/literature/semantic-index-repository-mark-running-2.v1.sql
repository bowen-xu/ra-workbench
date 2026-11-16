update embedding_records
         set status = 'running', error_code = null,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = (select embedding_record_id from embedding_outbox_jobs where id = ?1)
           and status in ('queued', 'failed', 'stale', 'running')