insert into evidence_links (id, claim_id, anchor_id, stance, rationale, created_at)
         values (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         on conflict(claim_id, anchor_id, stance) do update set rationale = excluded.rationale
