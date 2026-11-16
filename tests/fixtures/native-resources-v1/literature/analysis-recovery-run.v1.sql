update ai_runs
         set status = 'needs_review', error_code = null, error_message = null,
             finished_at = coalesce(finished_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1