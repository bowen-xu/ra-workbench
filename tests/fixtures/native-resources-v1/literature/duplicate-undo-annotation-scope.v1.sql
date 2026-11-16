select annotation.value from json_each(?1) annotation
where json_extract(annotation.value, '$.targetKind') = 'statement'
  and exists (
    select 1 from json_each(?2) statement
    where json_extract(statement.value, '$.id') = json_extract(annotation.value, '$.targetId')
  )
order by annotation.key