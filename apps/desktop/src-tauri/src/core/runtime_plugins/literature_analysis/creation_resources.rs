//! Original creation and paper-conversation statements, verified for one selected package.
use super::{conversation_store::ConversationStatements, creation_store::CreationStatements};
use crate::runtime_plugins::{
    RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::load_text,
};
pub(crate) struct CreationResources {
    latest: String,
    conversation: ConversationResources,
    supersede: String,
    create: String,
    turn: String,
}
impl CreationResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        Ok(Self {
            latest: load_text(
                catalog,
                package,
                "literature_library",
                "data/analysis-history-latest.v1.sql",
                "26d43786d1905051ee8818a6f1494f6c89d09857691cb9cc76970957aa9be5e4",
            )?,
            conversation: ConversationResources::load(catalog, package)?,
            supersede: load_text(
                catalog,
                package,
                "literature_library",
                "data/analysis-workflow-supersede.v1.sql",
                "c35ec79c02d27823a97063629869cdbe497738dafb790c8397171b2e4c302dcc",
            )?,
            create: load_text(
                catalog,
                package,
                "literature_library",
                "data/analysis-workflow-create.v1.sql",
                "6fd144aa49c5b9fb19f641dbdc17e37e94a80fc4e1d7ae201bfbe71464f9b876",
            )?,
            turn: load_text(
                catalog,
                package,
                "literature_library",
                "data/analysis-workflow-create-turn.v1.sql",
                "e1777d5ae3066bc099ac7c14796d47141676d6ec9f19ff688002a58532cf1ab2",
            )?,
        })
    }
    pub(crate) fn creation(&self) -> CreationStatements<'_> {
        CreationStatements {
            latest: &self.latest,
            supersede: &self.supersede,
            create: &self.create,
            turn: &self.turn,
        }
    }
    pub(crate) fn conversation(&self) -> ConversationStatements<'_> {
        self.conversation.statements()
    }
}

pub(crate) struct ConversationResources {
    active: String,
    pointer: String,
    title: String,
    set_pointer: String,
}
impl ConversationResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        Ok(Self {
            active: load_text(
                catalog,
                package,
                "literature_library",
                "data/analysis-workflow-generation-active.v1.sql",
                "6002731e630ec5a78b0936cba99bf3727727ccdd4d3c85f72431fb9fb6b42bc7",
            )?,
            pointer: load_text(
                catalog,
                package,
                "literature_library",
                "data/analysis-workflow-generation-pointer.v1.sql",
                "26869cede9fbdea82bb355be3fa7e10d818900ee8782bc89232cef1441bf854c",
            )?,
            title: load_text(
                catalog,
                package,
                "literature_library",
                "data/analysis-workflow-generation-title.v1.sql",
                "dd64107e82954da4eb46e68c5d0dba019e5288f52b9966ca0197c5a29ffc1024",
            )?,
            set_pointer: load_text(
                catalog,
                package,
                "literature_library",
                "data/analysis-workflow-generation-set-pointer.v1.sql",
                "33adc1254a3fcc74a4568c6929fadefdb2394ce52819095adbbc4020f12e48bf",
            )?,
        })
    }
    pub(crate) fn statements(&self) -> ConversationStatements<'_> {
        ConversationStatements {
            active: &self.active,
            pointer: &self.pointer,
            title: &self.title,
            set_pointer: &self.set_pointer,
        }
    }
}
