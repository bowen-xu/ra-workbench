update embedding_records
         set status = 'ready', lance_dataset_version = ?2, vector_norm = ?3,
             error_code = null, stale_at = null,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and status = 'running'