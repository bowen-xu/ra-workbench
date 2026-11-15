/** Existing public Core contracts; availability is separate from runtime authority. */
export const coreCapabilities = ["core.workbench", "core.workbench.source-workspace-host", "core.workbench.writing-frontend", "core.workbench.writing-runtime", "core.sdk.literature-shared", "core.structured-resources.write-diagnostics", "core.notes", "core.notes.workspace-host", "core.editor", "core.hbs", "core.agent", "core.resources",
  "core.pdf", "core.models", "core.docling", "core.git", "core.platform", "core.platform.confirmation",
  "core.agent.skills", "core.agent.tools", "core.agent.mcp", "core.agent.writing-dispatch-admission", "core.agent.writing-compaction-retirement"].map(id => ({id, version: 1}));
