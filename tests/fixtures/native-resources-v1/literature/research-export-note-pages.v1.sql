select id, title, parent_page_id, position, status, is_favorite, revision,
                     schema_version, created_at, updated_at, trashed_at
              from note_pages order by id
