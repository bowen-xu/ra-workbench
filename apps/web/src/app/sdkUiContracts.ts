/** Compile-time conformance only. Runtime modules remain Core-owned and are
 * provided to plugin packages by the workbench, not by a copied implementation. */
type Compatible<Expected, Actual extends Expected> = Actual;

export type SdkUiContracts = {
  readonly layout: Compatible<typeof import("@research-assistant/workbench-sdk/layout"), typeof import("../core/public/layout")>;
  readonly reorder: Compatible<typeof import("@research-assistant/workbench-sdk/reorder"), typeof import("../core/public/reorder")>;
  readonly overlay: Compatible<typeof import("@research-assistant/workbench-sdk/overlay"), typeof import("../core/public/overlay")>;
  readonly workspaceState: Compatible<typeof import("@research-assistant/workbench-sdk/workspace-state"), typeof import("../core/public/workspaceState")>;
  readonly platform: Compatible<typeof import("@research-assistant/workbench-sdk/platform"), typeof import("../core/public/platform")>;
  readonly feedback: Compatible<typeof import("@research-assistant/workbench-sdk/feedback"), typeof import("../core/public/feedback")>;
  readonly configuration: Compatible<typeof import("@research-assistant/workbench-sdk/configuration-hooks"), typeof import("../core/public/configuration")>;
  readonly agentContext: Compatible<typeof import("@research-assistant/workbench-sdk/agent-context"), typeof import("../core/public/agentContext/AgentContextProvider")>;
  readonly sourceNavigation: Compatible<typeof import("@research-assistant/workbench-sdk/source-navigation"), typeof import("../core/public/agentContext/sourceNavigation")>;
  readonly domainCapabilities: Compatible<typeof import("@research-assistant/workbench-sdk/domain-capabilities"), typeof import("../core/public/domainCapabilities")>;
  readonly noteDiagnostics: Compatible<typeof import("@research-assistant/workbench-sdk/note-diagnostics"), typeof import("../core/public/noteDiagnostics")>;
  readonly noteStorage: Compatible<typeof import("@research-assistant/workbench-sdk/note-storage"), typeof import("../core/public/noteStorage")>;
  readonly noteTransfer: Compatible<typeof import("@research-assistant/workbench-sdk/note-transfer"), typeof import("../core/public/noteTransfer")>;
};
