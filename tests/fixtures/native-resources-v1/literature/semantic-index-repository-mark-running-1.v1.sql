update embedding_outbox_jobs
         set status = 'running', lease_owner = ?2,
             lease_expires_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+5 minutes'),
             started_at = coalesce(started_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
             finished_at = null, error_code = null
         where id = ?1 and status in ('queued', 'failed', 'interrupted', 'running')