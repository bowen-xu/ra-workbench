select entry.value from json_each(?1) entry
where json_extract(entry.value, '$.itemId') = ?2
order by entry.key