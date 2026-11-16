select a.id, a.display_name, c.mime_type, c.byte_size, c.sha256
             from attachments a join content_objects c on c.sha256 = a.content_sha256
             where a.item_id = ?1 order by a.created_at