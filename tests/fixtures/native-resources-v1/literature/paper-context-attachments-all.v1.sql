select id, display_name, content_sha256 from attachments
             where item_id = ?1 and attachment_kind = 'pdf'
             order by created_at, id