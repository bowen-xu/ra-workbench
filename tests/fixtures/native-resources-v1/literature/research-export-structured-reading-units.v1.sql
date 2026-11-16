select id, project_id, paper_id, unit_kind, title, body, provenance_kind,
                     epistemic_role, revision, created_at, updated_at
              from structured_reading_units order by id
