use super::*;
fn id(note: &PendingNote) -> &str {
    &note
        .candidate
        .as_ref()
        .or(note.baseline.as_ref())
        .unwrap()
        .page_id
}
fn targets(blocks: &[NoteDocumentBlock]) -> Vec<String> {
    blocks
        .iter()
        .flat_map(|block| {
            let mut result = targets(&block.children);
            if block.block_type == "childPage"
                && let Some(target) = block.content["targetId"].as_str()
            {
                result.push(target.into());
            }
            result
        })
        .collect()
}
pub(super) fn remove(blocks: &mut Vec<NoteDocumentBlock>, page: &str) {
    blocks.retain(|block| !(block.block_type == "childPage" && block.content["targetId"] == page));
    for block in blocks {
        remove(&mut block.children, page);
    }
}
fn label(blocks: &mut [NoteDocumentBlock], page: &str, title: &str) {
    for block in blocks {
        if block.block_type == "childPage" && block.content["targetId"] == page {
            block.content["labelSnapshot"] = json!(title);
        }
        label(&mut block.children, page, title);
    }
}
fn child_block(blocks: &[NoteDocumentBlock], page: &str) -> Option<NoteDocumentBlock> {
    blocks.iter().find_map(|block| {
        if block.block_type == "childPage" && block.content["targetId"] == page {
            Some(block.clone())
        } else {
            child_block(&block.children, page)
        }
    })
}
fn ensure(
    backend: &NoteBackend,
    db: &Connection,
    group: &mut Vec<PendingNote>,
    page: &str,
) -> Result<usize, String> {
    if let Some(index) = group.iter().position(|note| id(note) == page) {
        return Ok(index);
    }
    backend.authorize(db, page, true)?;
    let stored = load(db, page)?;
    if stored.status != "active" {
        return Err("related note is not active".into());
    }
    let path = canonical(&stored);
    group.push(PendingNote {
        path: path.clone(),
        original_path: path,
        scope_request_id: backend.record.request_id,
        baseline: Some(stored.clone()),
        candidate: Some(stored),
        saved: None,
        review_document: Some(
            serde_json::from_value(crate::notes::hbs::read(db, page)?["baseline"].clone())
                .map_err(|e| e.to_string())?,
        ),
        review_metadata: None,
        reviewed_metadata: false,
        manual_edited: false,
        imports: vec![],
    });
    Ok(group.len() - 1)
}
pub(super) fn expand(
    backend: &NoteBackend,
    db: &Connection,
    group: &mut Vec<PendingNote>,
) -> Result<(), String> {
    fn modernize(blocks: &mut [NoteDocumentBlock], before: &[NoteDocumentBlock]) {
        for block in blocks {
            let retained = child_block(before, block.content["targetId"].as_str().unwrap_or(""))
                .is_some_and(|old| old.id == block.id);
            if block.block_type == "childPage" && !retained {
                block.block_type = "researchObjectReference".into();
                block.props["pageEntry"] = json!(true);
            }
            modernize(&mut block.children, before);
        }
    }
    for note in group.iter_mut() {
        let before = note
            .saved
            .as_ref()
            .or(note.baseline.as_ref())
            .map(|page| page.document.blocks.clone())
            .unwrap_or_default();
        if let Some(page) = &mut note.candidate {
            modernize(&mut page.document.blocks, &before);
        }
    }
    // Creation is the same independent Note Page + page-link entrance command
    // as the UI. The source is an insertion location, not lifetime parentage.
    // Its entrance is established immediately; only generated body text is B/H
    // review material. Never produce another legacy childPage here.
    let creations = group
        .iter()
        .enumerate()
        .filter_map(|(index, note)| {
            let page = note.candidate.as_ref()?;
            if note.baseline.is_some() {
                return None;
            }
            Some((
                index,
                page.parent_id.clone()?,
                page.page_id.clone(),
                page.title.clone(),
            ))
        })
        .collect::<Vec<_>>();
    for (index, source, page, title) in creations {
        let source_index = ensure(backend, db, group, &source)?;
        let entrance = NoteDocumentBlock {
            id: Uuid::new_v4().to_string(),
            block_type: "researchObjectReference".into(),
            props: json!({"pageEntry":true}),
            content: json!({"kind":"reference","targetKind":"note_page","targetId":page,"labelSnapshot":title,"quoteSnapshot":""}),
            children: vec![],
        };
        let mut accepted = super::super::note_review::baseline_document(&group[source_index]);
        accepted.blocks.push(entrance.clone());
        let parent = group[source_index]
            .candidate
            .as_mut()
            .ok_or("source page is proposed for deletion")?;
        parent.document.blocks.push(entrance);
        group[source_index].review_document = Some(accepted);
        group[index].candidate.as_mut().unwrap().parent_id = None;
    }
    // A removed child-page block means trash, except when this group moves that page.
    let changes = group.clone();
    let mut deleted = vec![];
    for change in &changes {
        if change.candidate.is_none() {
            deleted.push(id(change).to_owned());
            continue;
        }
        if let Some(before) = &change.baseline {
            let after = targets(&change.candidate.as_ref().unwrap().document.blocks);
            for child in targets(&before.document.blocks) {
                if !after.contains(&child)
                    && !group.iter().any(|note| {
                        note.candidate.as_ref().is_some_and(|page| {
                            crate::notes::page_entry_lifecycle::targets(&page.document.blocks)
                                .contains(&child)
                        })
                    })
                    && !group.iter().any(|note| {
                        id(note) == child
                            && note.candidate.as_ref().is_some_and(|note| {
                                note.parent_id.as_deref() != Some(&before.page_id)
                            })
                    })
                {
                    deleted.push(child);
                }
            }
        }
    }
    for page in deleted {
        let mut statement=db.prepare("with recursive subtree(id) as(select ?1 union all select p.id from note_pages p join subtree s on p.parent_page_id=s.id where p.status='active') select id from subtree").map_err(|error|error.to_string())?;
        let subtree = statement
            .query_map([&page], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        for page in subtree {
            let index = ensure(backend, db, group, &page)?;
            group[index].candidate = None;
        }
    }
    let changes = group.clone();
    for change in &changes {
        let page = id(change);
        let old = change
            .baseline
            .as_ref()
            .and_then(|note| note.parent_id.clone());
        let new = change
            .candidate
            .as_ref()
            .and_then(|note| note.parent_id.clone());
        if old != new || change.candidate.is_none() {
            if let Some(parent) = old {
                let index = ensure(backend, db, group, &parent)?;
                let mut accepted = super::super::note_review::baseline_document(&group[index]);
                remove(&mut accepted.blocks, page);
                if let Some(parent) = &mut group[index].candidate {
                    remove(&mut parent.document.blocks, page);
                }
                group[index].review_document = Some(accepted);
            }
            if let Some(parent) = new {
                let index = ensure(backend, db, group, &parent)?;
                let existing = group
                    .iter()
                    .filter_map(|note| note.baseline.as_ref())
                    .find_map(|note| child_block(&note.document.blocks, page));
                let parent = group[index]
                    .candidate
                    .as_mut()
                    .ok_or("destination parent is proposed for deletion")?;
                if !targets(&parent.document.blocks)
                    .iter()
                    .any(|target| target == page)
                {
                    parent.document.blocks.push(existing.unwrap_or_else(||NoteDocumentBlock{id:Uuid::new_v4().to_string(),block_type:"researchObjectReference".into(),props:json!({"pageEntry":true}),content:json!({"kind":"reference","targetKind":"note_page","targetId":page,"labelSnapshot":change.candidate.as_ref().map(|note|note.title.as_str()).unwrap_or(""),"quoteSnapshot":""}),children:vec![]}));
                }
            }
        }
        if let (Some(before), Some(after)) = (&change.baseline, &change.candidate)
            && before.title != after.title
            && let Some(parent) = &after.parent_id
        {
            let index = ensure(backend, db, group, parent)?;
            if let Some(parent) = &mut group[index].candidate {
                label(&mut parent.document.blocks, page, &after.title);
            }
        }
    }
    // Explicit child-page insertions also propose a reparenting of their target.
    let changes = group.clone();
    for change in changes {
        let Some(parent) = change.candidate else {
            continue;
        };
        for child in targets(&parent.document.blocks) {
            let actual = if let Some(note) = group.iter().find(|note| id(note) == child) {
                note.candidate
                    .as_ref()
                    .ok_or("child page is proposed for deletion")?
                    .parent_id
                    .clone()
            } else {
                db.query_row(
                    "select parent_page_id from note_pages where id=?1 and status='active'",
                    [&child],
                    |row| row.get::<_, Option<String>>(0),
                )
                .map_err(|_| "child page is unavailable")?
            };
            if actual.as_deref() != Some(&parent.page_id) {
                return Err(
                    "child-page target belongs to another parent; use Move to to relocate it"
                        .into(),
                );
            }
        }
    }
    // Saving a parent also normalizes the order of child pages. Include any
    // changed child metadata in the same authorization/version/undo group.
    let changes = group.clone();
    for change in changes {
        let Some(parent) = change.candidate else {
            continue;
        };
        for (position, child) in targets(&parent.document.blocks).into_iter().enumerate() {
            let current = group
                .iter()
                .find(|note| id(note) == child)
                .and_then(|note| note.candidate.as_ref())
                .cloned()
                .or_else(|| load(db, &child).ok())
                .ok_or("child metadata unavailable")?;
            if current.position != position as i64 {
                let index = ensure(backend, db, group, &child)?;
                group[index]
                    .candidate
                    .as_mut()
                    .ok_or("child proposed for deletion")?
                    .position = position as i64;
            }
        }
    }
    for change in group.iter() {
        let Some(candidate) = &change.candidate else {
            continue;
        };
        let mut parent = candidate.parent_id.clone();
        let mut visited = std::collections::BTreeSet::from([candidate.page_id.clone()]);
        while let Some(page) = parent {
            if !visited.insert(page.clone()) {
                return Err("page hierarchy would contain a cycle".into());
            }
            parent = if let Some(note) = group.iter().find(|note| id(note) == page) {
                note.candidate
                    .as_ref()
                    .ok_or("parent is proposed for deletion")?
                    .parent_id
                    .clone()
            } else {
                load(db, &page)?.parent_id
            };
            if visited.len() > 128 {
                return Err("page hierarchy depth limit".into());
            }
        }
    }
    if group.len() > 128 {
        return Err("related note change group exceeds 128 pages".into());
    }
    // Entrances can change another resource's lifecycle. Authorize and include
    // that resource in the same group even when its body is unchanged. Do not
    // infer this write permission from the ordinary reference read permission.
    let affected = group
        .iter()
        .flat_map(|note| {
            let before = note
                .saved
                .as_ref()
                .or(note.baseline.as_ref())
                .map(|p| crate::notes::page_entry_lifecycle::targets(&p.document.blocks))
                .unwrap_or_default();
            let after = note
                .candidate
                .as_ref()
                .map(|p| crate::notes::page_entry_lifecycle::targets(&p.document.blocks))
                .unwrap_or_default();
            before
                .symmetric_difference(&after)
                .cloned()
                .collect::<Vec<_>>()
        })
        .collect::<std::collections::BTreeSet<_>>();
    for page in affected {
        if group.iter().any(|note| id(note) == page) {
            continue;
        }
        let stored = load(db, &page)?;
        if stored.status == "trashed" {
            backend.authorize_restore(db, &page)?;
        } else {
            backend.authorize(db, &page, true)?;
        }
        let path = canonical(&stored);
        let baseline =
            serde_json::from_value(crate::notes::hbs::read(db, &page)?["baseline"].clone())
                .map_err(|e| e.to_string())?;
        group.push(PendingNote {
            path: path.clone(),
            original_path: path,
            scope_request_id: backend.record.request_id,
            baseline: Some(stored.clone()),
            candidate: Some(stored),
            saved: None,
            review_document: Some(baseline),
            review_metadata: None,
            reviewed_metadata: false,
            manual_edited: false,
            imports: vec![],
        });
    }
    Ok(())
}
