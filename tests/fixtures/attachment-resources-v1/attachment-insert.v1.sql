insert into attachments (id, item_id, content_sha256, display_name, source_path, created_at)
         values (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))