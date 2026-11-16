select receipt_json from browser_capture_attachments
             where scope = ?1 and session_id = ?2 order by external_id
