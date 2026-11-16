update claims set project_id = ?2, statement = ?3, epistemic_role = ?4,
                 revision = ?5,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1
