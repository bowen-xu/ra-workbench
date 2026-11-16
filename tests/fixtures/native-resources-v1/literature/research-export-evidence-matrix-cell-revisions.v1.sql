select id, cell_id, revision, snapshot_json, editor_kind, change_reason, created_at
              from evidence_matrix_cell_revisions order by cell_id, revision
