select request_sha256, receipt_json from browser_capture_attachments
         where scope = ?1 and session_id = ?2 and external_id = ?3
