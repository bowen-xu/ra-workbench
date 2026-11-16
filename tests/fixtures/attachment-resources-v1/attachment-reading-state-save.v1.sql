insert into reading_states (item_id, attachment_id, page_index, zoom, view_state_json, updated_at)
         values (?1, ?2, max(?3, 0), max(?4, 0.1), ?5, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         on conflict(attachment_id) do update set item_id = excluded.item_id,
             page_index = excluded.page_index, zoom = excluded.zoom,
             view_state_json = excluded.view_state_json, updated_at = excluded.updated_at