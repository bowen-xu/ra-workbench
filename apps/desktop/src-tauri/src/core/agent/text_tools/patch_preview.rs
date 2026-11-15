//! Bounded historical display from the exact prepared edit, never a later file read.
use super::*;

pub(super) fn preview(change: &TextChange) -> Value {
    let before = change
        .before
        .as_ref()
        .map(|v| v.text.replace("\r\n", "\n"))
        .unwrap_or_default();
    let after = change.after.as_deref().unwrap_or("").replace("\r\n", "\n");
    let mut hunks = Vec::new();
    if change.before.is_none() || change.after.is_none() {
        let (text, marker) = if change.before.is_none() {
            (&after, '+')
        } else {
            (&before, '-')
        };
        hunks.push(json!({"oldStart":1,"newStart":1,"lines":text.lines().map(|line|format!("{marker}{line}")).collect::<Vec<_>>()}));
    } else {
        let mut delta = 0isize;
        for (index, edit) in change.edits.iter().enumerate() {
            let previous = index
                .checked_sub(1)
                .map(|i| change.edits[i].end)
                .unwrap_or(0);
            let next = change
                .edits
                .get(index + 1)
                .map(|v| v.start)
                .unwrap_or(before.len());
            let start = before[..edit.start]
                .rmatch_indices('\n')
                .nth(3)
                .map(|(i, _)| i + 1)
                .unwrap_or(0)
                .max(previous);
            let end = before[edit.end..]
                .match_indices('\n')
                .nth(2)
                .map(|(i, _)| edit.end + i + 1)
                .unwrap_or(before.len())
                .min(next);
            let old_start = before[..start].bytes().filter(|c| *c == b'\n').count() + 1;
            let removed = &before[edit.start..edit.end];
            let mut lines = Vec::new();
            for line in before[start..edit.start].lines() {
                lines.push(format!(" {line}"));
            }
            for line in removed.lines() {
                lines.push(format!("-{line}"));
            }
            for line in edit.replacement.lines() {
                lines.push(format!("+{line}"));
            }
            for line in before[edit.end..end].lines() {
                lines.push(format!(" {line}"));
            }
            hunks.push(json!({"oldStart":old_start,"newStart":old_start.saturating_add_signed(delta),"lines":lines}));
            delta += edit.replacement.lines().count() as isize - removed.lines().count() as isize;
        }
    }
    let mut value = json!({"path":change.path,"name":change.before.as_ref().and_then(|v|v.name.as_ref()),"hunks":hunks});
    // Never add a full deleted document or an unbounded preview to model history.
    let row_count: usize = hunks
        .iter()
        .map(|hunk| hunk["lines"].as_array().map_or(0, Vec::len))
        .sum();
    if value.to_string().len() > 48_000 || row_count > 600 {
        value["hunks"] = json!([]);
        value["omitted"] = json!(true);
    }
    value
}

/// A repaired draft is an editing surface, never the saved-change baseline.
pub(super) fn saved_structured(
    path: &str,
    name: &str,
    before: Option<&Value>,
    after: Option<&Value>,
) -> Value {
    let mut value = json!({"path":path,"name":name,"comparison":"saved","beforeSaved":before,"afterSaved":after});
    let rows = [before, after]
        .into_iter()
        .flatten()
        .map(|content| {
            serde_json::to_string_pretty(content)
                .map_or(usize::MAX / 2, |text| text.lines().count())
        })
        .sum::<usize>();
    if value.to_string().len() > 48_000 || rows > 600 {
        if let Some(object) = value.as_object_mut() {
            object.remove("beforeSaved");
            object.remove("afterSaved");
        }
        value["omitted"] = json!(true);
    }
    value
}
pub(super) fn applied(previews: &[Value], results: &[TargetResult]) -> Vec<Value> {
    let mut chosen = previews
        .iter()
        .filter(|preview| {
            results.iter().any(|result| {
                result.status == TargetStatus::Applied && preview["path"] == result.path
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    for result in results
        .iter()
        .filter(|result| result.status == TargetStatus::Applied)
    {
        if let Ok(feedback) = serde_json::from_str::<Value>(&result.message)
            && let Some(saved) = feedback
                .get("savedDiff")
                .filter(|saved| saved["comparison"] == "saved" && saved["path"] == result.path)
        {
            if let Some(preview) = chosen
                .iter_mut()
                .find(|preview| preview["path"] == result.path)
            {
                *preview = saved.clone();
            } else {
                chosen.push(saved.clone());
            }
        }
    }
    let mut used = 0;
    chosen
        .iter()
        .map(|preview| {
            let mut value = preview.clone();
            if used + value.to_string().len() > 128_000 {
                value["hunks"] = json!([]);
                if let Some(object) = value.as_object_mut() {
                    object.remove("beforeSaved");
                    object.remove("afterSaved");
                }
                value["omitted"] = json!(true);
            }
            used += value.to_string().len();
            value
        })
        .collect()
}
