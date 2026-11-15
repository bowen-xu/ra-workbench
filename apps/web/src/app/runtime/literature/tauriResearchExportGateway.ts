import type {AppRuntime} from "../../../core/runtime/appRuntime";
import {invoke} from "../../../core/runtime/platformCore";
import {createResearchExportGateway} from "@research-assistant/literature-library-plugin/researchExport";

export const tauriResearchExportGateway = createResearchExportGateway({
  exportAll: targetDirectory => invoke("export_research_knowledge", {request: {targetDirectory}}),
  cancel: () => invoke("cancel_research_export"),
});

/** Research export already uses the selected preview transport. Unlike the
 * other literature ports it must keep that existing preview capability. */
export function configureResearchExport(runtime: AppRuntime): void {
  if (runtime.mode !== "browser") runtime.gateways.getResearchExportGateway ??= tauriResearchExportGateway;
}
