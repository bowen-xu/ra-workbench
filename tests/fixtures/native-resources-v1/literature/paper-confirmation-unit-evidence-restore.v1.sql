insert into reading_unit_evidence_links
             (id, reading_unit_id, anchor_id, stance, rationale, created_at)
             values (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
