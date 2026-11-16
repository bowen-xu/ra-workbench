insert into reading_unit_template_fields (reading_unit_id, portrait_candidate_id, field_id, created_at)
                values (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
