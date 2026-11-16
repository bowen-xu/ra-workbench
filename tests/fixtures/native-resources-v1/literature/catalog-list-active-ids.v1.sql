select id from bibliographic_items
                where archived_at is null and trashed_at is null order by created_at desc