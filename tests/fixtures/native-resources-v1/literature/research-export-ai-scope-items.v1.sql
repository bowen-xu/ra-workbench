select id, run_id, ordinal, item_kind, object_id, parent_object_id,
                     content_sha256, object_revision, snapshot_hash, created_at
              from ai_scope_items order by run_id, ordinal
