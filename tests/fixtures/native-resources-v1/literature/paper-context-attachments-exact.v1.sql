select id, display_name, content_sha256 from attachments
                     where id = ?1 and item_id = ?2 and attachment_kind = 'pdf'