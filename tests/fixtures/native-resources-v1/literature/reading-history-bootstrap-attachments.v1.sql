select attachment_id, content_sha256, parse_run_id
         from paper_context_attachments where snapshot_id = ?1 order by ordinal
