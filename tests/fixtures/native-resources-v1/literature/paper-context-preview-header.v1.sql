select s.paper_id,s.context_hash,s.context_schema_version from paper_context_snapshots s
         join bibliographic_items p on p.id=s.paper_id and p.trashed_at is null where s.id=?1