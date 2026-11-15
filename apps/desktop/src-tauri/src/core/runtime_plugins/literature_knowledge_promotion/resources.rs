//! Exact original statements, resolved before the original review transaction.
use crate::runtime_plugins::{
    RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data,
};
use std::collections::BTreeMap;
const FOUNDATION_ASSETS: &[(&str, &str)] = &[
    (
        "concept-foundations-scope.v1.sql",
        "fd1e9c368c820b45c18eb052eca4a514338c103442a6d287954fb9c28e678d36",
    ),
    (
        "concept-foundations-insert.v1.sql",
        "09251aba2b52e05ef9e83c4fc0e966181da025616142c2256bc20f898d912d6f",
    ),
    (
        "concept-foundations-papers.v1.sql",
        "2672050979721370c9f3e2d6578e871e622cd1efc6c15fb6cccd04d9cb6d5e3c",
    ),
    (
        "concept-foundations-sources.v1.sql",
        "1cd9e6eae2460231a3d1de8c75c9d65896b4e900d40f62608df6e88d6babc616",
    ),
];
const ALGORITHM_ASSETS: &[(&str, &str)] = &[
    (
        "concept-algorithm-evolution-exists.v1.sql",
        "c43c68192a281713dc3a07978157c19488eae93eee86522ee06293ebf75a393a",
    ),
    (
        "concept-algorithm-evolution-papers.v1.sql",
        "739ed01c0ea8db069777d34099b519423c6bd73fb7684c19f7011eb6d6278fc0",
    ),
    (
        "concept-algorithm-evolution-insert.v1.sql",
        "486f4f6c2800863819d962005d0f29af695e1cbc965a1a6f5f31bdd6486473db",
    ),
    (
        "concept-algorithm-evolution-source.v1.sql",
        "11317383e9eacdb26d1691f5535dfef9b47ccb7490b0c60e380e6904c326db9b",
    ),
    (
        "concept-algorithm-run-insert.v1.sql",
        "b6ebed5859a73b8d700ccf5747dddc6263727f3724cb2ef8265bdf109345da42",
    ),
    (
        "concept-algorithm-run-finish.v1.sql",
        "bdf3d07ed6e9c29c0644f4621b5c1fab9e49d2abacfc11e2ebf569fd13368ce8",
    ),
    (
        "concept-algorithm-run-fail.v1.sql",
        "91d9d6ca7a8c7443f0c12cae07efdab97d801c0f55b1920f83ff93e6309a15f0",
    ),
    (
        "concept-algorithm-run-read.v1.sql",
        "7a315f6f824de6a1dff2609c61c64dbbb158f72dce0ad0dccc9667be45be8fec",
    ),
    (
        "concept-algorithm-run-evolution.v1.sql",
        "da0427206890f6f6c101e4fae11ab123264d4558d5ab8243f24071508355175d",
    ),
    (
        "concept-algorithm-run-foundations.v1.sql",
        "51f6e4c25593b3336138a77e687c62805875bc8b67ea725449ffffb60fe11e33",
    ),
    (
        "concept-algorithm-run-concept-watermark.v1.sql",
        "5ed111e60240fddd72077942b18c4ee1fcaf17c444ad89d8df34e39ce5ae791b",
    ),
    (
        "concept-algorithm-run-relation-watermark.v1.sql",
        "9ed8b1c56d479ac691235fc73776c2dc78b705aacc3477f721a273c16b0f3e16",
    ),
];
const IDENTITY_ASSETS: &[(&str, &str)] = &[
    (
        "concept-identity-mapping-insert.v1.sql",
        "6b639709d067d13aac07b85f02980cb50384358bad9e14e156ca36bd791c68d4",
    ),
    (
        "concept-identity-mapping-delete.v1.sql",
        "1d17befd545784e0e603f6a2891414a25d74c057407b20c19750e738e73e7816",
    ),
    (
        "concept-identity-events-list.v1.sql",
        "bcab48df0a796551403b7fc05f66a9fc8661154ea663b39a25f620f1feb1c949",
    ),
    (
        "concept-identity-mapping-exists.v1.sql",
        "ba2f2d6970b7773c6100d0873c8891d192c9514b77968c6bd86ebfd22e6cc1cd",
    ),
    (
        "concept-identity-children-exist.v1.sql",
        "01bfa5094668d38b90811a302216c17a57648ff27d03adb2cac17a9d660608f6",
    ),
    (
        "concept-identity-summary.v1.sql",
        "deb617f52b2cd6d9df2f91dfeadd7911457c20ecc63d7f458c0e0bd22cb57a7c",
    ),
    (
        "concept-identity-relations.v1.sql",
        "c1e4f06388af6077132468c4f92fd8fe4c23b9c2eb67f25d82591aeb8ead3e55",
    ),
    (
        "concept-identity-event-insert.v1.sql",
        "5b3f31535d2a8bb833e2fd91c5627f45d6e3c72bbda5a2fc8d5b4626c6648907",
    ),
    (
        "concept-identity-impact-insert.v1.sql",
        "b00a46c7646e77a916c67fd2103d9c3331be2ed0ff579c841f5142074be5445c",
    ),
    (
        "concept-identity-event-read.v1.sql",
        "50d51fe5ef453059ed2e0a7f42173d3edd002220aa6bdd3995c280ad946db7d4",
    ),
    (
        "concept-identity-active-sources.v1.sql",
        "b0e7b80f03c5de25c7e526e24b34345864dc0b2f690ef6ea92b84a051fad6c2a",
    ),
    (
        "concept-identity-event-relations.v1.sql",
        "2112cb2b84ad137b1a49229f2a787945b3b97930c9e5efaf686bee2cd546554d",
    ),
    (
        "concept-identity-jobs-cancel.v1.sql",
        "b5d6b568ed8460f4010a66e8f9cac6c4b34c479469e277d3e401ac771abd9a0b",
    ),
    (
        "concept-identity-records-tombstone.v1.sql",
        "89d30636ca694a34ae1c39524480137e1e9e0141a48d1fc72e4dc29480cd73ee",
    ),
];
const RELATION_ASSETS: &[(&str, &str)] = &[
    (
        "relation-node-read-concept.v1.sql",
        "652a431e31010bfe6fd3d5da5e59b933fc65dc3d370d32e07a0ce9606bfbcea1",
    ),
    (
        "relation-node-search-concept.v1.sql",
        "6b1f90cc094abf1a92b8c140fabec80c620475cb105124561f1247936b167b66",
    ),
    (
        "relation-node-read-definition_claim.v1.sql",
        "2e0a86a31e68b258353a89f3b6f4ce7609914c8055edf982475586ccd8acb216",
    ),
    (
        "relation-node-search-definition_claim.v1.sql",
        "21f6566b6f1fb0433bbd106b82b96d51fc675de014ccb9e966c865a250a870dd",
    ),
    (
        "relation-node-read-reading_unit.v1.sql",
        "0384d4ef7da1efc8d049bad78ee64c51fe0cb752d901130f50053cbb38ac91ba",
    ),
    (
        "relation-node-search-reading_unit.v1.sql",
        "9a2c0f42a6f5bdd987f8a8d32002d0b92ee85f59e80ea44454d49ab75d308571",
    ),
    (
        "relation-node-read-paper.v1.sql",
        "beda0e5ced9769e97c1ced797cfb9881b9678137832d50ffd04a9024b2413880",
    ),
    (
        "relation-node-search-paper.v1.sql",
        "35cbd6a0dad3255bbe9915b764dfacb079fe6c429e1f8a46cd93d52ea8f7ba9e",
    ),
    (
        "relation-node-read-note_page.v1.sql",
        "2632dc010a10467b5fb3dac739b6063e6163829626b82fc7d5e684cec70cd916",
    ),
    (
        "relation-node-search-note_page.v1.sql",
        "380303f7ba39d761e5d569708c17bc3fe564559b17bbeed1c9c4d9f6327e76c3",
    ),
    (
        "relation-manual-list.v1.sql",
        "43cf12d03378214151ecafdfe8d85ce2634ecd8ca64baebc0eedc7982399f504",
    ),
    (
        "relation-manual-insert.v1.sql",
        "cdf17777214a6d036362b81ce80707cf27b57675f24b584c5f208ee08970bcc1",
    ),
    (
        "relation-manual-update.v1.sql",
        "5aedf559b378c099d2a4404c513326a6163cb8c46b4c9646555818e1bda81f93",
    ),
    (
        "relation-manual-read.v1.sql",
        "2837e72fb251c41236a9608091966f187ac5d3db07c01031679ad1b555412465",
    ),
    (
        "relation-manual-evidence-insert.v1.sql",
        "09d12b90fb61495803127b8240e25e9036bb43b57231fe4368e47f7105ff0527",
    ),
];
const MANUAL_ASSETS: &[(&str, &str)] = &[
    (
        "definition-manual-current-revision.v1.sql",
        "4578de031322ce28154c54f4a0e76a1d5c3564734813f07411a397fe4c5ab42b",
    ),
    (
        "definition-manual-update.v1.sql",
        "bcde8d6ed34381972fcdbe99998b0bc746f3b746b46a371d3f46e7e2ebfdb01a",
    ),
    (
        "definition-manual-list.v1.sql",
        "b4cad6b88c1391115956d07881ee6dbcdd4fb136498f471e1e61d41bb5a6f957",
    ),
    (
        "knowledge-manual-revisions.v1.sql",
        "c74f7857dad623225f4923fd8b5e4f25c2a5920b29c35e0c71711669c8e4c141",
    ),
];
const CAPTURE_ASSETS: &[(&str, &str)] = &[(
    "concept-capture-anchor-quote.v1.sql",
    "ba3f1938dceb10860c7e65b3f5f51aab74c42ba7d8c56ae7ee85f199f728b4f6",
)];
const PAPER_ASSETS: &[(&str, &str)] = &[
    (
        "paper-concept-assertion-insert.v1.sql",
        "2d0b32a8ac9027052371040ef16bd496878d18987f216567852a1da4681855e7",
    ),
    (
        "paper-concept-assertion-revision-insert.v1.sql",
        "840e64c153c02d580dbf37d60114d38f8cd2d9c8a45fac7a63f3f90ca0619611",
    ),
];
const ASSETS: &[(&str, &str)] = &[
    (
        "nal-bootstrap-term-read.v1.sql",
        "c7401da24c9cfd310c074ae3c610e2c8fbc95e35a2927231f98eaaeef8f9b835",
    ),
    (
        "nal-bootstrap-term-lookup.v1.sql",
        "2b6e609217711a1011a5eed9354b245cfcbc6e4ec58aed3956bace8f65e3268c",
    ),
    (
        "nal-bootstrap-term-insert.v1.sql",
        "b6e981361443d5dc5c5cef997cc929d9a96039c860ed0c05a52ee91a8c4e4210",
    ),
    (
        "nal-bootstrap-concept-main.v1.sql",
        "3a613a786d32c719a2ad941b176c4ffcfe97a46e2312d6573e140cf89c35f69a",
    ),
    (
        "nal-bootstrap-concept-terms.v1.sql",
        "38476a062382d4025f4d3357248a1c8d9a04667fe4434fdd5da4a3dfc9a3a2ec",
    ),
    (
        "nal-bootstrap-concept-insert.v1.sql",
        "17aa3373a8143fcbc5b7d6de362839fbcc4aad0657cb8325b7ddd20f1ef6c03b",
    ),
    (
        "nal-bootstrap-concept-preferred-insert.v1.sql",
        "9e4fb36230362a46f1dd3ab5c26d70db40ed1df689419d876a22f056684f7ed1",
    ),
    (
        "knowledge-promotion-alias-insert.v1.sql",
        "7208c729cbbd99f48d6be01ce9be046b1a72beb1947a59b007e944ed7ab9cbe7",
    ),
    (
        "knowledge-promotion-concept-revision-update.v1.sql",
        "a90136833c3e42bee01be02a3fa6b4f56f0457a7bcd0874cae9401a3aa4402a7",
    ),
    (
        "knowledge-promotion-canonical-concept.v1.sql",
        "385754f47681866ec27b378b3efb499535d19c140d1924e29aa38908d91574bc",
    ),
    (
        "knowledge-promotion-term-usage-read.v1.sql",
        "1ce16ae8541079d31c5579a78dda81b9668991bab550740dc1a68df04df7cf9b",
    ),
    (
        "knowledge-promotion-definition-read.v1.sql",
        "3a9ab7a4750a0719ba5861327cd5de3cb5098720be2b6ecc519504385f2bbfb3",
    ),
    (
        "knowledge-promotion-definition-insert.v1.sql",
        "cfd245a3d90dfd8ffd3f1c5b6e239e4d0cd24cf8ad81f8e400eee0e1ccbd2b1f",
    ),
    (
        "knowledge-promotion-definition-evidence-insert.v1.sql",
        "f85a08ad6cfa3e874ff06c29cc77acc3d3cf17f4c7c2ab0cfcb782959b596a79",
    ),
    (
        "nal-bootstrap-research-revision-insert.v1.sql",
        "1a31986a2e74182d3f5a70491fec1feda7cde5cd3cefd8d4ffb5723608393e9d",
    ),
    (
        "knowledge-promotion-project-exists.v1.sql",
        "9d7fb0dfc0ce5b4d24176d3483315a697c03e8746a27538258ec72f17c086620",
    ),
    (
        "knowledge-promotion-relation-insert.v1.sql",
        "754015311662ab8a77c444a9d5242136889ae82cf6e4022671bf04af86ad8183",
    ),
    (
        "knowledge-promotion-relation-evidence-insert.v1.sql",
        "55162d55a2e18d0106942da61cef07cfaac9705327f0467947227de4b2d5cc46",
    ),
    (
        "knowledge-promotion-term-usage-lookup.v1.sql",
        "1ccf52f3109cc9f1a70d86bb041b7064b312d976750af3f5158cbb8c313acd01",
    ),
    (
        "knowledge-promotion-term-usage-insert.v1.sql",
        "c7382b77a4c7dd7a768ba35943e1061e23265eac38d6ca8e14b1780e1dece129",
    ),
    (
        "knowledge-promotion-relation-concept-node.v1.sql",
        "652a431e31010bfe6fd3d5da5e59b933fc65dc3d370d32e07a0ce9606bfbcea1",
    ),
    (
        "knowledge-promotion-anchor-exists.v1.sql",
        "2b782245169ef7d1328c9d8ab280398b9f8fa7152088573a3ff4893b3d62c4e0",
    ),
    (
        "nal-bootstrap-concept-name-available.v1.sql",
        "d9fe641dcbd8d32f2144038223db91d174f833e2d61e64f3629a67da525f742b",
    ),
];
pub(crate) struct KnowledgeSql(BTreeMap<&'static str, String>);
impl KnowledgeSql {
    #[cfg(test)]
    pub(crate) fn for_test(statements: &[(&'static str, &str)]) -> Self {
        Self(
            statements
                .iter()
                .map(|&(name, sql)| (name, sql.to_owned()))
                .collect(),
        )
    }

    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let mut statements = BTreeMap::new();
        for &(name, sha) in ASSETS {
            statements.insert(
                name,
                data::load_text(
                    catalog,
                    package,
                    "literature_library",
                    &format!("data/{name}"),
                    sha,
                )?,
            );
        }
        Ok(Self(statements))
    }
    pub(crate) fn load_paper(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let mut sql = Self::load(catalog, package)?;
        for &(name, sha) in PAPER_ASSETS {
            sql.0.insert(
                name,
                data::load_text(
                    catalog,
                    package,
                    "literature_library",
                    &format!("data/{name}"),
                    sha,
                )?,
            );
        }
        Ok(sql)
    }
    pub(crate) fn load_capture(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let mut sql = Self::load(catalog, package)?;
        for &(name, sha) in CAPTURE_ASSETS {
            sql.0.insert(
                name,
                data::load_text(
                    catalog,
                    package,
                    "literature_library",
                    &format!("data/{name}"),
                    sha,
                )?,
            );
        }
        Ok(sql)
    }
    pub(crate) fn load_manual(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let mut sql = Self::load(catalog, package)?;
        for &(name, sha) in MANUAL_ASSETS {
            sql.0.insert(
                name,
                data::load_text(
                    catalog,
                    package,
                    "literature_library",
                    &format!("data/{name}"),
                    sha,
                )?,
            );
        }
        Ok(sql)
    }
    pub(crate) fn load_relation(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let mut sql = Self::load(catalog, package)?;
        for &(name, sha) in RELATION_ASSETS {
            sql.0.insert(
                name,
                data::load_text(
                    catalog,
                    package,
                    "literature_library",
                    &format!("data/{name}"),
                    sha,
                )?,
            );
        }
        Ok(sql)
    }
    pub(crate) fn load_identity(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let mut sql = Self::load(catalog, package)?;
        for &(name, sha) in IDENTITY_ASSETS {
            sql.0.insert(
                name,
                data::load_text(
                    catalog,
                    package,
                    "literature_library",
                    &format!("data/{name}"),
                    sha,
                )?,
            );
        }
        Ok(sql)
    }
    pub(crate) fn load_algorithm(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let mut sql = Self::load(catalog, package)?;
        for &(name, sha) in ALGORITHM_ASSETS {
            sql.0.insert(
                name,
                data::load_text(
                    catalog,
                    package,
                    "literature_library",
                    &format!("data/{name}"),
                    sha,
                )?,
            );
        }
        Ok(sql)
    }
    pub(crate) fn load_foundations(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let mut sql = Self::load_algorithm(catalog, package)?;
        for &(name, sha) in FOUNDATION_ASSETS {
            sql.0.insert(
                name,
                data::load_text(
                    catalog,
                    package,
                    "literature_library",
                    &format!("data/{name}"),
                    sha,
                )?,
            );
        }
        Ok(sql)
    }
    pub(crate) fn has_paper(&self) -> bool {
        self.0.contains_key("paper-concept-assertion-insert.v1.sql")
    }

    pub(super) fn statement(&self, key: &str) -> &str {
        self.0
            .get(key)
            .expect("only original fixed knowledge statements are selected")
            .trim_end_matches('\n')
    }
}
