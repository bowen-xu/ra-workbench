select distinct c.sha256, c.byte_size
             from attachments a
             join content_objects c on c.sha256 = a.content_sha256
             where a.item_id = ?1