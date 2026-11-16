select id, created_at from evidence_links
             where claim_id = ?1 and anchor_id = ?2 and stance = ?3
