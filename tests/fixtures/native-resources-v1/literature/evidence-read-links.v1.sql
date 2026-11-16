select id, anchor_id, stance, rationale, created_at from evidence_links
             where claim_id = ?1 order by created_at, id
