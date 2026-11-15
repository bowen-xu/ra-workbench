use super::*;
use std::collections::{HashMap, HashSet};

impl ResponsesContinuation {
    /// Complete only pending calls whose results are already durable in the
    /// application journal. No tool execution or synthetic success is permitted.
    pub(crate) fn with_durable_tool_results(
        mut self,
        results: &[Value],
    ) -> Result<Self, ModelError> {
        let checkpoint = self
            .state
            .get_mut()
            .map_err(|_| ModelError::Unavailable)?
            .as_mut()
            .ok_or(ModelError::InvalidRequest)?;
        if checkpoint.origin_scope_id.is_some() {
            return Err(ModelError::InvalidRequest);
        }
        let mut pending = HashSet::new();
        let mut used = HashSet::new();
        let mut completed = HashMap::new();
        for item in &checkpoint.normalized_prefix {
            match item["type"].as_str() {
                Some("function_call") => {
                    let id = item["call_id"]
                        .as_str()
                        .ok_or(ModelError::InvalidStream)?
                        .to_owned();
                    if !used.insert(id.clone()) {
                        return Err(ModelError::InvalidStream);
                    }
                    pending.insert(id);
                }
                Some("function_call_output") => {
                    let id = item["call_id"]
                        .as_str()
                        .ok_or(ModelError::InvalidStream)?
                        .to_owned();
                    if !pending.remove(&id) {
                        return Err(ModelError::InvalidStream);
                    }
                    completed.insert(id, item.clone());
                }
                // A response can place assistant commentary between function
                // items. Only their paired durable results clear pending calls.
                Some("message") if item["role"] == "assistant" => {}
                _ if !pending.is_empty() => return Err(ModelError::InvalidStream),
                _ => {}
            }
        }
        let mut supplied = HashSet::new();
        for result in results {
            let id = result["call_id"]
                .as_str()
                .ok_or(ModelError::InvalidStream)?;
            if result["type"] != "function_call_output"
                || result["output"].as_str().is_none()
                || !supplied.insert(id)
            {
                return Err(ModelError::InvalidStream);
            }
            if let Some(existing) = completed.get(id) {
                if existing != result {
                    return Err(ModelError::InvalidStream);
                }
            } else if pending.remove(id) {
                checkpoint.normalized_prefix.push(result.clone());
                checkpoint.wire_prefix.push(result.clone());
            } else {
                return Err(ModelError::InvalidStream);
            }
        }
        if !pending.is_empty() {
            return Err(ModelError::InvalidRequest);
        }
        self.encode_checkpoint()?;
        Ok(self)
    }

    /// Transfer a confirmed checkpoint from a closed turn into a new owner. Only the
    /// normalized comparison IDs change; provider payload and source text do not.
    #[cfg(test)]
    pub(crate) fn inherit_for_turn(
        mut self,
        new_scope: Uuid,
        aliases: &HashMap<String, String>,
    ) -> Result<Self, ModelError> {
        if new_scope.is_nil() || new_scope == self.scope_id {
            return Err(ModelError::InvalidRequest);
        }
        let state = self.state.get_mut().map_err(|_| ModelError::Unavailable)?;
        let checkpoint = state.as_mut().ok_or(ModelError::InvalidRequest)?;
        if checkpoint.origin_scope_id.is_some() {
            return Err(ModelError::InvalidRequest);
        }
        remap_pairs(checkpoint, aliases)?;
        checkpoint.origin_scope_id = Some(self.scope_id);
        self.scope_id = new_scope;
        self.encode_checkpoint()?;
        Ok(self)
    }
}

pub(super) fn remap_pairs(
    checkpoint: &mut Checkpoint,
    aliases: &HashMap<String, String>,
) -> Result<(), ModelError> {
    let mut calls = HashSet::new();
    let mut results = HashSet::new();
    for item in &mut checkpoint.normalized_prefix {
        let seen = match item["type"].as_str() {
            Some("function_call") => &mut calls,
            Some("function_call_output") => &mut results,
            _ => continue,
        };
        let id = item["call_id"].as_str().ok_or(ModelError::InvalidStream)?;
        if let Some(alias) = aliases.get(id) {
            if !seen.insert(id.to_owned()) || alias.is_empty() {
                return Err(ModelError::InvalidStream);
            }
            item["call_id"] = Value::String(alias.clone());
        }
    }
    if calls.len() != aliases.len() || results.len() != aliases.len() {
        return Err(ModelError::InvalidStream);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_checkpoint_upgrades_and_inheritance_requires_complete_alias_pairs() {
        let scope = Uuid::new_v4();
        let model = ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "model".into(),
            revision: None,
        };
        let call =
            json!({"type":"function_call","call_id":"native","name":"read","arguments":"{}"});
        let result = json!({"type":"function_call_output","call_id":"native","output":"evidence"});
        let raw = json!({"schema_version":1,"scope_id":scope,"model":model,"credential_scope":Uuid::from_u128(1),
            "checkpoint":{"request_id":Uuid::new_v4(),"actual_model":model,
                "normalized_prefix":[call,result],"wire_prefix":[call,result]}});
        let aliases = HashMap::from([("native".into(), "call_stable".into())]);
        let load = |value: &Value| {
            ResponsesContinuation::restore(scope, model.clone(), 16384, &value.to_string()).unwrap()
        };
        let mut pending = raw.clone();
        pending["checkpoint"]["normalized_prefix"] = json!([call]);
        pending["checkpoint"]["wire_prefix"] = json!([call]);
        assert!(load(&pending).with_durable_tool_results(&[]).is_err());
        let completed = load(&pending)
            .with_durable_tool_results(std::slice::from_ref(&result))
            .unwrap();
        let completed_json: Value =
            serde_json::from_str(&completed.encode_checkpoint().unwrap()).unwrap();
        assert_eq!(completed_json["checkpoint"]["wire_prefix"][1], result);
        let mut changed = result.clone();
        changed["output"] = json!("different evidence");
        assert!(load(&raw).with_durable_tool_results(&[changed]).is_err());
        assert!(
            load(&pending)
                .with_durable_tool_results(&[result.clone(), result.clone()])
                .is_err()
        );
        let inherited = load(&raw)
            .inherit_for_turn(Uuid::new_v4(), &aliases)
            .unwrap();
        assert_eq!(inherited.saved_request_id().unwrap(), None);
        let encoded = inherited.encode_checkpoint().unwrap();
        let value: Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(value["schema_version"], 2);
        assert_eq!(
            value["checkpoint"]["normalized_prefix"][0]["call_id"],
            "call_stable"
        );
        assert_eq!(value["checkpoint"]["wire_prefix"][0]["call_id"], "native");
        assert!(
            inherited
                .inherit_for_turn(Uuid::new_v4(), &aliases)
                .is_err()
        );
        let mut missing = raw.clone();
        missing["checkpoint"]["normalized_prefix"]
            .as_array_mut()
            .unwrap()
            .pop();
        assert!(
            load(&missing)
                .inherit_for_turn(Uuid::new_v4(), &aliases)
                .is_err()
        );
        assert!(load(&raw).inherit_for_turn(scope, &aliases).is_err());
    }
}
