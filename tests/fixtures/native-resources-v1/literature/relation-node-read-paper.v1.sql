select id, case when title = '' then '未命名文献'
                                 when length(title) > 1000 then substr(title, 1, 999) || '…'
                                 else title end,
                        item_type
                 from bibliographic_items where id = ?1 and trashed_at is null
