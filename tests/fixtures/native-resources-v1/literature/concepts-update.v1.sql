update concepts set label = ?2, explanation = ?3, disambiguation = ?4,
                 domain = ?5, tradition = ?6, period = ?7, revision = ?8,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1