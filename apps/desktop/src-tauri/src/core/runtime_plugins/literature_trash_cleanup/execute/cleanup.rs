use super::*;
use crate::data_interface::plugin_resources::PreparedDataWrite;
use crate::runtime_plugins::literature_trash_cleanup::resources::call;

pub(super) fn remove(
    tx: &Transaction<'_>,
    resources: &Resources,
    checked: &CheckedCleanupPlan,
    item: &str,
) -> std::result::Result<(), (&'static str, Problem)> {
    let plan = checked.ready().map_err(|p| ("cleanup-plan", p))?;
    resources
        .write(tx, "root", text(item))
        .map_err(|p| ("scope-root", p))?;
    for table in &plan.soft_owners {
        resources
            .write(
                tx,
                &format!("soft-{table}"),
                vec![DataValue::Text(item.into()), DataValue::Text(table.clone())],
            )
            .map_err(|p| ("scope-soft", p))?;
    }
    {
        // Preserve prepare-all before executing any propagation, after scope seeds.
        let mut statements = plan
            .relations
            .iter()
            .map(|relation| {
                let resource = resources
                    .resource_id(&relation.id)
                    .map_err(|p| ("collect-prepare", p))?;
                PreparedDataWrite::prepare_in(
                    tx,
                    resource,
                    call(
                        resource,
                        vec![
                            DataValue::Text(item.into()),
                            DataValue::Text(relation.child.clone()),
                            DataValue::Text(relation.parent.clone()),
                        ],
                    ),
                )
                .map_err(|p| ("collect-prepare", Problem::Data(p)))
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        loop {
            let mut added = 0u64;
            for statement in &mut statements {
                added += statement
                    .execute()
                    .map_err(|p| ("collect", Problem::Data(p)))?;
            }
            if added == 0 {
                break;
            }
        }
    }
    for (name, parent_index) in resources.post_specs() {
        let mut arguments = text(item);
        if let Some(index) = parent_index {
            let parent = plan
                .delete_tables
                .get(index)
                .ok_or(("post-scope", Problem::Binding))?;
            arguments.push(DataValue::Text(parent.clone()));
        }
        resources
            .write(tx, name, arguments)
            .map_err(|p| ("post-scope", p))?;
    }
    for table in &plan.delete_tables {
        resources
            .write(
                tx,
                &format!("delete-{table}"),
                vec![DataValue::Text(item.into()), DataValue::Text(table.clone())],
            )
            .map_err(|p| ("owned-delete", p))?;
    }
    Ok(())
}
