//! Closed execution profiles, never a runtime method/permission registry.
use research_workbench_native_sdk::{
    literature_trace_protocol as literature, protocol as writing,
    writing_projects_protocol as projects,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeProfile {
    WritingPreview,
    WritingProjects,
    NoteTraceDecision,
}

impl NativeProfile {
    pub(crate) fn plugin_id(self) -> &'static str {
        match self {
            Self::WritingPreview => writing::PLUGIN_ID,
            Self::WritingProjects => projects::PLUGIN_ID,
            Self::NoteTraceDecision => literature::PLUGIN_ID,
        }
    }
    pub(crate) fn capability(self) -> &'static str {
        match self {
            Self::WritingPreview => writing::CAPABILITY_ID,
            Self::WritingProjects => projects::CAPABILITY_ID,
            Self::NoteTraceDecision => literature::CAPABILITY_ID,
        }
    }
    pub(crate) fn protocol_version(self) -> u32 {
        match self {
            Self::WritingPreview => writing::PROTOCOL_VERSION,
            Self::WritingProjects => projects::PROTOCOL_VERSION,
            Self::NoteTraceDecision => literature::PROTOCOL_VERSION,
        }
    }
    pub(crate) fn executable_name(self) -> &'static str {
        match self {
            Self::WritingPreview | Self::WritingProjects => "writing-projects-host",
            Self::NoteTraceDecision => "literature-library-host",
        }
    }
    pub(crate) fn max_frame_bytes(self) -> usize {
        match self {
            Self::WritingPreview => writing::MAX_FRAME_BYTES,
            Self::WritingProjects => projects::MAX_FRAME_BYTES,
            Self::NoteTraceDecision => literature::MAX_FRAME_BYTES,
        }
    }
}
