select exists(select 1 from evidence_matrices
         where id = ?1 and project_id = ?2 and current_revision = ?3)
