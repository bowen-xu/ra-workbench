select id, title, '笔记页面' from note_pages
         where status = 'active' and (?1 = '' or instr(lower(title), lower(?1)) > 0)
         order by updated_at desc limit 15
