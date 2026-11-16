select id, paper_id, revision, profile_refs_json, change_reason, created_at
              from paper_reading_profile_selection_revisions order by paper_id, revision
