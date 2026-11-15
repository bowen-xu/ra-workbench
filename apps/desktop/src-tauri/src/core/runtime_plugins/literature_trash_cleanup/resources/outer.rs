use super::*;
pub(super) fn load(
    catalog: &RuntimePluginCatalog,
    package: &RuntimePluginPackage,
    statements: &mut BTreeMap<String, AcceptedDataResource>,
) -> Result<(), RuntimePluginError> {
    {
        let resource = super::load(
            catalog,
            package,
            "expired-ids",
            "37bbb4d3b5f440c0a49631e816fe92ef78ddbd3179ce39806c1616cf8bf140ab",
            DataAccess::Read,
            vec![TEXT; 0],
            vec![TEXT],
        )?;
        statements.insert(resource.grant().id.clone(), resource);
    }
    {
        let resource = super::load(
            catalog,
            package,
            "item-exists",
            "80173affeca72358c1f958dbd4ec8d9504d17be12244789698f38cdc0ef79d1b",
            DataAccess::Read,
            vec![TEXT; 1],
            vec![INTEGER],
        )?;
        statements.insert(resource.grant().id.clone(), resource);
    }
    {
        let resource = super::load(
            catalog,
            package,
            "deletion-impact",
            "2b5366a7de8420bed5af4f4724afd017840ac352b9f07a9e040b6b2eae22c339",
            DataAccess::Read,
            vec![TEXT; 1],
            vec![INTEGER, INTEGER, INTEGER, INTEGER, INTEGER, INTEGER],
        )?;
        statements.insert(resource.grant().id.clone(), resource);
    }
    {
        let resource = super::load(
            catalog,
            package,
            "content-objects",
            "899e593e065f0734d9b4521698678150c9ef0b38d3878f6a1206efc10932221f",
            DataAccess::Read,
            vec![TEXT; 1],
            vec![TEXT, INTEGER],
        )?;
        statements.insert(resource.grant().id.clone(), resource);
    }
    {
        let resource = super::load(
            catalog,
            package,
            "fts-delete",
            "715c7a5add6de4717ac9e65b14e27cfabe82ede482fda066a46ae900837ea9fe",
            DataAccess::Write,
            vec![TEXT; 1],
            vec![],
        )?;
        statements.insert(resource.grant().id.clone(), resource);
    }
    {
        let resource = super::load(
            catalog,
            package,
            "item-delete",
            "6351ed616e968ef619a1c149e5942d514c2db1836b7af315a1d4698121bbac5f",
            DataAccess::Write,
            vec![TEXT; 1],
            vec![],
        )?;
        statements.insert(resource.grant().id.clone(), resource);
    }
    {
        let resource = super::load(
            catalog,
            package,
            "gc-scheduled",
            "171d66e24c80a9146022d870be9313bbe864f2d7f95b8732e256d627fe156b2a",
            DataAccess::Read,
            vec![TEXT; 1],
            vec![INTEGER],
        )?;
        statements.insert(resource.grant().id.clone(), resource);
    }
    {
        let resource = super::load(
            catalog,
            package,
            "orphan-creators-delete",
            "46d28c4db84c18d27d8fe4fab81690321c19eb44d6863a7a2e64d508f4b42afe",
            DataAccess::Write,
            vec![TEXT; 0],
            vec![],
        )?;
        statements.insert(resource.grant().id.clone(), resource);
    }
    {
        let resource = super::load(
            catalog,
            package,
            "orphan-tags-delete",
            "abf09a1db1329b541b482791491f3115480cf9a539baf2b58d7780279aab97b0",
            DataAccess::Write,
            vec![TEXT; 0],
            vec![],
        )?;
        statements.insert(resource.grant().id.clone(), resource);
    }
    Ok(())
}
