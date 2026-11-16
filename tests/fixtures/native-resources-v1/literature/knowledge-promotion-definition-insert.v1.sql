insert into definition_claims
         (id, concept_id, statement, scope_note, provenance_kind, epistemic_role,
          revision, created_at, updated_at)
         values (?1, ?2, ?3, ?4, ?5, 'definition', 1,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
