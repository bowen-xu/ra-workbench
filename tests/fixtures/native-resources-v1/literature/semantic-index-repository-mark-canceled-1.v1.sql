update embedding_outbox_jobs
         set status = 'canceled', lease_owner = null, lease_expires_at = null,
             error_code = 'canceled', finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and status in ('queued', 'running')