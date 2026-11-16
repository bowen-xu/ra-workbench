select c.family_name, c.given_name, c.literal_name, ic.role
             from item_creators ic join creators c on c.id = ic.creator_id
             where ic.item_id = ?1 order by ic.position