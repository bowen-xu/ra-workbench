select id from semantic_representations
             where subject_kind = 'concept' and subject_id = ?1 and subject_revision = ?2
               and representation_kind = 'concept_identity' and language = ?3
               and render_version = ?4