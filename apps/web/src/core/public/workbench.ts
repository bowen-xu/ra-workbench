/** Workbench UI and host callbacks. No application runtime, raw IPC or database access. */
export type { WorkspaceView, WorkspaceLocation, WorkspacePresentation, WorkspaceTab, WorkspaceClosePort } from "../contracts/workbench";
export { WorkspaceTabsProvider, WorkspaceTabBar, usePublishWorkspaceTabs } from "../components/workspace-tabs/WorkspaceTabs";
export { WorkspacePresentationProvider, useWorkspacePresentation } from "../components/workspace-tabs/WorkspacePresentation";
export { AppWindowToolbar } from "../components/AppWindowToolbar";
export { ControlHint } from "../components/ControlHint";
export { getReaderCloseCoordinator, ReaderCloseCoordinator } from "../workspace/DocumentCloseCoordinator";
export type { ReaderCloseBoundary } from "../workspace/DocumentCloseCoordinator";
export { WorkbenchLayout } from "../components/workbench/WorkbenchLayout";
export type { WorkbenchLayoutProps } from "../components/workbench/WorkbenchLayout";
