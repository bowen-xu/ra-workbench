insert into derived_research_views
         (id, project_id, view_kind, title, scope_note, inputs_json, items_json,
          review_state, provenance_kind, provider_id, model_id, generated_at,
          current_revision, created_at, updated_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'accepted', ?8, ?9, ?10, ?11, 1,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
