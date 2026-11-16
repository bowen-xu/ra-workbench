select statement.value from json_each(?1) statement
where exists (
    select 1 from json_tree(json_extract(statement.value, '$.subject')) term
    where case when term.type = 'object' then json_extract(term.value, '$.kind') end = 'paper'
      and case when term.type = 'object' then json_extract(term.value, '$.id') end = ?2
) or exists (
    select 1 from json_tree(json_extract(statement.value, '$.predicate')) term
    where case when term.type = 'object' then json_extract(term.value, '$.kind') end = 'paper'
      and case when term.type = 'object' then json_extract(term.value, '$.id') end = ?2
)
order by statement.key