-- Preserve pre-existing homonyms; prevent every new name collision at the database boundary.
create index concepts_name_lookup on concepts(trim(label));
create trigger concepts_unique_name_insert before insert on concepts
when exists (select 1 from concepts where trim(label) = trim(new.label))
begin
    select raise(abort, '概念名称已存在，请使用不同名称，或用 ` 添加语境');
end;
create trigger concepts_unique_name_update before update of label on concepts
when trim(new.label) <> trim(old.label)
  and exists (select 1 from concepts where id <> old.id and trim(label) = trim(new.label))
begin
    select raise(abort, '概念名称已存在，请使用不同名称，或用 ` 添加语境');
end;
