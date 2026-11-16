insert into relation_assertion_evidence_links
             (id, relation_assertion_id, anchor_id, stance, rationale, created_at)
             values (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
             on conflict(relation_assertion_id, anchor_id, stance) do nothing
