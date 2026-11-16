update embedding_records
         set status = 'queued', error_code = null, stale_at = null,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and status in ('failed', 'stale', 'queued')