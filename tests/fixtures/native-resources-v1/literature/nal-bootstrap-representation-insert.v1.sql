insert into semantic_representations
             (id, subject_kind, subject_id, subject_revision, representation_kind,
              namespace, language, render_version, rendered_text, input_sha256, created_at)
             values (?1, 'concept', ?2, ?3, 'concept_identity', 'accepted_concepts',
                     ?4, ?5, ?6, ?7, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))