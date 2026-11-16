insert into browser_capture_attachments
         (scope, session_id, external_id, item_id, attachment_id, request_sha256,
          metadata_json, content_sha256, receipt_json, created_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
