select id, case when title = '' then '未命名文献'
                         when length(title) > 1000 then substr(title, 1, 999) || '…'
                         else title end,
                item_type from bibliographic_items
         where trashed_at is null and (?1 = '' or instr(lower(title), lower(?1)) > 0)
         order by updated_at desc limit 15
