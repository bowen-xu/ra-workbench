select id, page_id, page_revision, schema_version, document_json, reason,
                     created_at, is_pinned from note_revisions order by page_id, page_revision
