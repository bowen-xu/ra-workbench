import type {RegisteredPluginView} from "../../../core/runtime/plugins/viewRegistry";
import type {WorkbenchComposition} from "../../workbenchComposition";
import type {RuntimePluginStatus} from "./runtimePluginWorkbench";

/** The existing finite library role, admitted through its actual owner. A
 * matching route name from another package does not supply this projection. */
export function libraryWorkspaceContribution(views: readonly RegisteredPluginView[]) {
  return views.find(view => view.pluginId === "literature_library" && view.id === "library");
}

/** Assembly availability only. Resource resolution and access still use Core. */
export function sourceDomainProvider(_composition: WorkbenchComposition,
  views: readonly RegisteredPluginView[], statuses: readonly RuntimePluginStatus[],
  domain: "literature_library" | "writing_projects", writingReady?: RegisteredPluginView): object | undefined {
  const registered = domain === "literature_library" ? libraryWorkspaceContribution(views)
    : views.find(view => view.pluginId === "writing_projects" && view.id === "writing");
  if (registered) return (domain !== "writing_projects" || registered === writingReady)
    && statuses.some(status => status.pluginId === domain && status.state === "ready")
    ? registered : undefined;
  return undefined;
}
