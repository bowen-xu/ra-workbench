update full_text_index_jobs set state = 'completed', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where attachment_id = ?1