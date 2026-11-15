import type {AppRuntime} from "../../../core/runtime/appRuntime";
import {getWritingGateway} from "@research-assistant/writing-projects-plugin/writingGateway";
import {tauriWritingGateway} from "./tauriWritingGateway";
import {tauriWritingCheckpointGateway} from "./tauriWritingCheckpointGateway";
import {tauriWritingNoteAssetGateway} from "./tauriWritingNoteAssetGateway";

export function configureWritingWorkbench(runtime: AppRuntime): void {
  if (runtime.mode === "preview" && !runtime.nativeAgentPreview) return;
  if (runtime.mode === "desktop" || runtime.nativeAgentPreview)
    runtime.gateways.getWritingCheckpointGateway ??= tauriWritingCheckpointGateway;
  if (runtime.mode === "desktop") runtime.gateways.getWritingNoteAssetGateway ??= tauriWritingNoteAssetGateway;
  if (!runtime.gateways.getWritingGateway) {
    if (runtime.mode === "desktop" || runtime.nativeAgentPreview) runtime.gateways.getWritingGateway = tauriWritingGateway;
    runtime.gateways.getWritingGateway = getWritingGateway();
  }
}
