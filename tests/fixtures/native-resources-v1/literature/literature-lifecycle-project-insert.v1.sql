insert into project_items (project_id, item_id, added_at)
             values (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))