update evidence_matrices set title = ?2, paper_ids_json = ?3,
             dimensions_json = ?4, current_revision = ?5,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and current_revision = ?6
