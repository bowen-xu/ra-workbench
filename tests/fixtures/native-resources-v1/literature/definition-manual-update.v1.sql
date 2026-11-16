update definition_claims set concept_id = ?2, statement = ?3, scope_note = ?4,
                 revision = ?5, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             where id = ?1
