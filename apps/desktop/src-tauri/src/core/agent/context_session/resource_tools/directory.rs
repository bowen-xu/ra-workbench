//! Authorized current inventory; source bodies are not loaded here.
use super::*;
impl ResourceTools {
    pub(super) fn entries(&self, db: &Connection) -> Result<Vec<Entry>, Error> {
        let mut entries = Vec::new();
        for source in &self.record.input.context {
            let start = entries.len();
            // Preserve composer reference IDs while using current saved file reads.
            if resource_grants::resource(source).is_none()
                && source.anchor.as_deref().and_then(|text|serde_json::from_str::<Value>(text).ok()).is_some_and(|anchor| {
                    anchor["contextOwner"]["kind"] == "manuscript" && self.record.input.context.iter().any(|grant|
                        matches!(resource_grants::resource(grant),Some(Resource::Manuscript {resource_id,new_member_access:Some(_),..}) if anchor["manuscriptId"] == resource_id))
                }) {
                let anchor:Value=serde_json::from_str(source.anchor.as_deref().ok_or(Error::InvalidInput)?).map_err(|_|Error::InvalidInput)?;
                let resource=Resource::ManuscriptFile {resource_id:Uuid::parse_str(anchor["manuscriptId"].as_str().ok_or(Error::InvalidInput)?).map_err(|_|Error::InvalidInput)?,path:source.resource_id.clone()};
                let mut identity = match resource_grants::grants_with_scope(db,std::slice::from_ref(&resource), self.writing_scope()) {
                    Ok(mut grants)=>grants.remove(0),
                    Err(Error::SourceUnavailable)=>continue,
                    Err(error)=>return Err(error),
                };
                identity.source_id=source.source_id.clone();
                let mut current:Value=serde_json::from_str(identity.anchor.as_deref().ok_or(Error::InvalidInput)?).map_err(|_|Error::InvalidInput)?;
                current["contextAccess"]=anchor["contextAccess"].clone();
                identity.anchor=Some(current.to_string());
                entries.push(Entry {identity,resource:Some(resource),owner:anchor["manuscriptId"].as_str().map(str::to_owned)});
                continue;
            }
            self.append(db, source, &mut entries)?;
            let scope = resource_grants::resource(source);
            let owner = scope.as_ref().and_then(|resource| {
                if let Resource::StructuredObject { resource_id, .. } = resource {
                    return crate::structured_resources::get_header(db, resource_id)
                        .ok()
                        .map(|header| header.paper_id.unwrap_or_else(|| resource_id.clone()));
                }
                serde_json::to_value(resource)
                    .ok()
                    .and_then(|resource| resource["resourceId"].as_str().map(str::to_owned))
            });
            for entry in &mut entries[start..] {
                entry.owner = owner.clone();
            }
        }
        let mut seen = std::collections::HashSet::new();
        entries.retain(|entry| {
            seen.insert(
                entry
                    .resource
                    .as_ref()
                    .and_then(|resource| serde_json::to_string(resource).ok())
                    .unwrap_or_else(|| entry.identity.source_id.clone()),
            )
        });
        if entries.len() > 100_000 {
            return Err(Error::Budget);
        }
        Ok(entries)
    }
    fn append(
        &self,
        db: &Connection,
        source: &ContextText,
        entries: &mut Vec<Entry>,
    ) -> Result<(), Error> {
        match resource_grants::resource(source) {
            Some(resource) if crate::agent::context_session::group_scope::is_dynamic(&resource) => {
                for part in crate::agent::context_session::group_scope::parts_with_scope(
                    db,
                    &resource,
                    self.writing_scope(),
                )?
                .iter()
                .filter(|part| part.selected)
                {
                    for mut identity in resource_grants::grants_with_scope(
                        db,
                        std::slice::from_ref(&part.resource),
                        self.writing_scope(),
                    )? {
                        let mut anchor: Value = serde_json::from_str(
                            identity.anchor.as_deref().ok_or(Error::InvalidInput)?,
                        )
                        .map_err(|_| Error::InvalidInput)?;
                        anchor["contextAccess"] = json!(part.access);
                        identity.anchor = Some(anchor.to_string());
                        self.append(db, &identity, entries)?;
                        // An explicit note rule overrides the notes-group default
                        // without persisting or returning a second inventory row.
                        if matches!(part.resource, Resource::Note { .. }) {
                            for entry in entries.iter_mut().filter(|entry| {
                                entry.resource.as_ref().is_some_and(|resource| {
                                    serde_json::to_value(resource).ok()
                                        == serde_json::to_value(&part.resource).ok()
                                })
                            }) {
                                let mut anchor: Value = serde_json::from_str(
                                    entry
                                        .identity
                                        .anchor
                                        .as_deref()
                                        .ok_or(Error::InvalidInput)?,
                                )
                                .map_err(|_| Error::InvalidInput)?;
                                anchor["contextAccess"] = json!(part.access);
                                entry.identity.anchor = Some(anchor.to_string());
                            }
                        }
                    }
                }
            }
            Some(Resource::PaperNotes {
                resource_id,
                excluded_note_ids,
            }) => {
                resource_grants::label_with_scope(
                    db,
                    &Resource::PaperNotes {
                        resource_id: resource_id.clone(),
                        excluded_note_ids: excluded_note_ids.clone(),
                    },
                    self.writing_scope(),
                )?;
                for (id, _) in
                    crate::agent::context_session::catalog::paper_note_pages(db, &resource_id)?
                {
                    if !excluded_note_ids.contains(&id) {
                        let resource = Resource::Note {
                            resource_id: Uuid::parse_str(&id).map_err(|_| Error::Unavailable)?,
                        };
                        let mut identity = resource_grants::grants_with_scope(
                            db,
                            std::slice::from_ref(&resource),
                            self.writing_scope(),
                        )?
                        .remove(0);
                        let parent: Value = serde_json::from_str(
                            source.anchor.as_deref().ok_or(Error::InvalidInput)?,
                        )
                        .map_err(|_| Error::InvalidInput)?;
                        let mut anchor: Value = serde_json::from_str(
                            identity.anchor.as_deref().ok_or(Error::InvalidInput)?,
                        )
                        .map_err(|_| Error::InvalidInput)?;
                        anchor["contextAccess"] = parent["contextAccess"].clone();
                        identity.anchor = Some(anchor.to_string());
                        entries.push(Entry {
                            identity,
                            resource: Some(resource),
                            owner: None,
                        });
                    }
                }
            }
            Some(Resource::PaperPortrait { resource_id }) => {
                let object = crate::structured_resources::list_headers(db, Some(&resource_id))
                    .map_err(|_| Error::SourceUnavailable)?
                    .into_iter()
                    .find(|resource| resource.kind == "portrait")
                    .ok_or(Error::SourceUnavailable)?;
                let parent: Value =
                    serde_json::from_str(source.anchor.as_deref().ok_or(Error::InvalidInput)?)
                        .map_err(|_| Error::InvalidInput)?;
                let access = serde_json::from_value(parent["contextAccess"].clone())
                    .map_err(|_| Error::InvalidInput)?;
                let resource = Resource::StructuredObject {
                    resource_id: object.id,
                    access: Some(access),
                };
                let identity = resource_grants::grants_with_scope(
                    db,
                    std::slice::from_ref(&resource),
                    self.writing_scope(),
                )?
                .remove(0);
                entries.push(Entry {
                    identity,
                    resource: Some(resource),
                    owner: Some(resource_id),
                });
            }
            Some(Resource::TemplateCatalog { resource_id, .. }) => {
                let label = resource_grants::label_with_scope(
                    db,
                    &Resource::TemplateCatalog {
                        resource_id: resource_id.clone(),
                        access: None,
                    },
                    self.writing_scope(),
                )?;
                let mut identity = source.clone();
                identity.text = json!({"label":label,"content":"Global template directory; Add File creates a private draft, publication requires user confirmation."}).to_string();
                entries.push(Entry {
                    identity,
                    resource: Some(Resource::TemplateCatalog {
                        resource_id: resource_id.clone(),
                        access: None,
                    }),
                    owner: Some(resource_id.clone()),
                });
                for resource in crate::structured_resources::list_headers(db, None)
                    .map_err(|_| Error::Unavailable)?
                    .into_iter()
                    .filter(|resource| {
                        resource.kind == "readingTemplate" && resource.paper_id.is_none()
                    })
                {
                    let owned = crate::structured_resources::has_owned_draft(
                        db,
                        &resource.id,
                        &self.record.conversation_id.to_string(),
                    )
                    .map_err(|_| Error::Unavailable)?;
                    if resource.revision == 0 && !owned {
                        continue;
                    }
                    let child = Resource::StructuredObject {
                        resource_id: resource.id,
                        access: Some(
                            if owned
                                && crate::agent::context_session::structured::access_catalog(
                                    &self.record,
                                    true,
                                )
                            {
                                sources::Access::ReadWrite
                            } else {
                                sources::Access::ReadOnly
                            },
                        ),
                    };
                    let identity = resource_grants::grants_with_scope(
                        db,
                        std::slice::from_ref(&child),
                        self.writing_scope(),
                    )?
                    .remove(0);
                    entries.push(Entry {
                        identity,
                        resource: Some(child),
                        owner: Some(resource_id.clone()),
                    });
                }
            }
            Some(resource) => {
                let label = resource_grants::label_with_scope(db, &resource, self.writing_scope())?;
                let mut identity = source.clone();
                identity.text = json!({"label":label,"resource":resource,"content":"Current authorized resource; body not loaded."}).to_string();
                entries.push(Entry {
                    identity,
                    resource: Some(resource),
                    owner: None,
                });
            }
            None => entries.push(Entry {
                identity: source.clone(),
                resource: None,
                owner: None,
            }),
        }
        Ok(())
    }
}
