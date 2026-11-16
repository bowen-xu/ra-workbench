insert into full_text_index_jobs (attachment_id, state, page_count, updated_at)
         values (?1, 'running', ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         on conflict(attachment_id) do update set state = 'running', page_count = excluded.page_count,
             error_message = null, updated_at = excluded.updated_at