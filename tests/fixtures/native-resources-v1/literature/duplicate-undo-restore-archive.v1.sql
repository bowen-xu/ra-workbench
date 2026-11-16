update bibliographic_items set archived_at = ?2,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1