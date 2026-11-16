select id, matrix_id, revision, snapshot_json, editor_kind, change_reason,
                     created_at from evidence_matrix_revisions order by matrix_id, revision
