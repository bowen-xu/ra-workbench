update embedding_outbox_jobs
         set status = 'interrupted', lease_owner = null, lease_expires_at = null,
             error_code = 'interrupted', finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and status = 'running'