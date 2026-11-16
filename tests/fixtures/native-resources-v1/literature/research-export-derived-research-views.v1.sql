select id, project_id, view_kind, title, scope_note, inputs_json, items_json,
                     review_state, provenance_kind, provider_id, model_id, generated_at,
                     current_revision, created_at, updated_at from derived_research_views order by id
