select id, candidate_id, candidate_revision, version, summary_text,
                     source_kind, is_selected, idempotency_key, created_at, discarded_at
              from ai_candidate_summary_versions
              order by candidate_id, candidate_revision, version
