select anchor_id, stance, rationale from reading_unit_evidence_links
             where reading_unit_id = ?1 order by created_at, id
