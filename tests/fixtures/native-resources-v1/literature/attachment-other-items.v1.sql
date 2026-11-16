select distinct item_id from attachments
             where content_sha256 = ?1 and item_id is not null and item_id <> ?2