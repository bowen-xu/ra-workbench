update embedding_outbox_jobs
         set status = 'succeeded', lease_owner = null, lease_expires_at = null,
             error_code = null, finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and status = 'running'