select a.content_sha256 from attachments a join bibliographic_items b on b.id=a.item_id
         where a.id=?2 and a.item_id=?1 and b.archived_at is null and b.trashed_at is null