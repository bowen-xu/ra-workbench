select exists(select 1 from concepts where deleted_at is null and trim(label)=?1 and (?2 is null or id<>?2))
         and not exists(select 1 from concepts where id=?2 and trim(label)=?1)