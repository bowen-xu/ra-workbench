select project_id, paper_id, unit_kind, title, body, provenance_kind,
                    epistemic_role, revision
             from structured_reading_units where id = ?1 and reverted_at is null
