insert into definition_claim_evidence_links
             (id, definition_claim_id, anchor_id, stance, rationale, created_at)
             values (?1, ?2, ?3, 'supports', '', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
             on conflict(definition_claim_id, anchor_id, stance) do nothing
