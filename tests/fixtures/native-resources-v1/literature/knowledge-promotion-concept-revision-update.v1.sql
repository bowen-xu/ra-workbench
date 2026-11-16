update concepts set revision = revision + 1,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1
