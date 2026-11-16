insert into browser_capture_sessions
         (scope, session_id, operation, request_sha256, payload_json, receipt_json, created_at)
         values (?1, ?2, ?3, ?4, ?5, '{}', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
