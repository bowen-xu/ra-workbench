insert into ai_candidate_recovery_audits
         (id, candidate_id, run_id, workflow_id, recovery_kind, recovery_reason,
          recovery_version, original_run_status, original_run_error_code,
          original_run_error_message, original_attempt_status, original_attempt_error_code,
          original_attempt_error_message, recovered_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))