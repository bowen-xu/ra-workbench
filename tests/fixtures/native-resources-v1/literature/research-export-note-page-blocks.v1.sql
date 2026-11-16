select id, page_id, parent_block_id, position, block_type, props_json,
                     content_json, plain_text, schema_version, revision, status, is_favorite,
                     created_at, updated_at, trashed_at from note_blocks
              where scope_kind = 'page' order by page_id, position, id
