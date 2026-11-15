import type {AppRuntime} from "../../../core/runtime/appRuntime";
import {getAiAssistanceTransport} from "../../../core/services/aiAssistanceGateway";
import {getDoclingGateway} from "../../../core/services/doclingGateway";
import {getLiteratureGateway} from "@research-assistant/literature-library-plugin/literatureGateway";
import {createPreparedPaperGenerationGateway} from "@research-assistant/literature-library-plugin/paperPreparation";
import {configureLiteratureServices} from "./configureLiteratureServices";
import {configureResearchExport} from "./tauriResearchExportGateway";

export function configureLiteratureWorkbench(runtime: AppRuntime): void {
  if (runtime.mode === "preview" && !runtime.nativeAgentPreview) return;
  configureResearchExport(runtime);
  const needsLiterature = runtime.mode !== "preview" && !runtime.gateways.getLiteratureGateway;
  configureLiteratureServices(runtime);
  if (needsLiterature) runtime.gateways.getLiteratureGateway = getLiteratureGateway();
  const literature = runtime.gateways.getLiteratureGateway;
  if (literature) runtime.gateways.doclingAttachmentMetadata ??= {
    async describe(itemId, attachmentId) {
      const item = await literature.get(itemId);
      return {paperTitle: item.title, fileName: item.attachments.find(value => value.id === attachmentId)?.displayName ?? "PDF"};
    },
  };
  runtime.gateways.getAiAssistanceGateway ??= createPreparedPaperGenerationGateway(getAiAssistanceTransport(), getDoclingGateway);
}
