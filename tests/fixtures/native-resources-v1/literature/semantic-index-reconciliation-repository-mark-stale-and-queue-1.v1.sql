update embedding_records
         set status = 'stale', error_code = 'lance_row_missing',
             stale_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and status != 'tombstoned'