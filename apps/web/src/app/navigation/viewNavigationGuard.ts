import type {WorkspacePresentation, WorkspaceView} from "../../core/contracts/workbench";
import type {PluginViewRegistry} from "../../core/runtime/plugins/viewRegistry";

/** Capture the actual registration before awaiting preservation. A reused ID
 * is a different target; Core document routes do not depend on a manager. */
export function captureViewNavigationGuard({view, presentation, registry}: {
  readonly view: WorkspaceView;
  readonly presentation: WorkspacePresentation;
  readonly registry?: Pick<PluginViewRegistry, "getSnapshot">;
}): () => boolean {
  const coreDocument = view === "notes" && presentation === "document";
  const registration = coreDocument ? undefined : registry?.getSnapshot().find(target => target.id === view);
  if (registration) return () => registry?.getSnapshot().includes(registration) === true;
  const coreRoute = view === "notes" || view === "settings";
  return () => coreRoute;
}
