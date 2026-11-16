select candidate.id, run.id, run.task_kind, candidate.candidate_kind,
                run.status, run.error_code, run.error_message, run.provider_id, run.model_id,
                run.context_snapshot_id, run.scope_hash, run.input_tokens, run.output_tokens,
                candidate.original_payload_hash, revision.payload_hash, revision.payload_json
         from ai_candidates candidate
         join ai_runs run on run.id = candidate.run_id
         join ai_candidate_revisions revision on revision.candidate_id = candidate.id
            and revision.revision = candidate.current_revision
         join ai_evidence_bundles bundle on bundle.id = candidate.evidence_bundle_id
         where candidate.validation_status = 'valid'
           and bundle.validation_status in ('valid', 'partial')
           and not exists(
               select 1 from ai_candidate_recovery_audits audit
               where audit.candidate_id = candidate.id
           )
           and (
               (run.status = 'failed' and run.error_code = 'storage_error')
               or run.status = 'validating'
           )
           and (
               (run.task_kind = 'paper_overview'
                and candidate.candidate_kind = 'paper_overview_unit_v1')
               or (run.task_kind = 'paper_concept_map'
                   and candidate.candidate_kind = 'paper_concept_map_v1')
           )
         order by candidate.created_at, candidate.id