select id from bibliographic_items
             where trashed_at is not null
               and trashed_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-14 days')
             order by trashed_at