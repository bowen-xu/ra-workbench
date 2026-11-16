update embedding_outbox_jobs
         set status = 'failed', lease_owner = null, lease_expires_at = null,
             error_code = ?2, finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and status in ('queued', 'running')