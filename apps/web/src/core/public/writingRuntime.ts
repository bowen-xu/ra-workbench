import {getAppRuntime} from "../runtime/appRuntime";
import type {WritingGateway} from "@research-assistant/data/writing";
import type {WritingCheckpointGateway} from "@research-assistant/data/writing";
import type {WritingNoteAssetGateway} from "@research-assistant/data/writing";

/** Read the existing App-selected writing capability, including retained runtime
 * bindings. Browser fallback stays with the plugin; native execution stays with App. */
export function resolveWritingRuntimeGateway(browserFallback: WritingGateway): WritingGateway {
  const runtime = getAppRuntime();
  const selected = runtime.gateways.getWritingGateway;
  if (selected) return selected;
  if (runtime.mode === "preview" && !runtime.nativeAgentPreview)
    throw new Error("预览未覆盖业务网关：getWritingGateway");
  if (runtime.mode === "desktop" || runtime.nativeAgentPreview)
    throw Object.assign(new Error("writing_gateway_unavailable：当前工作台尚未接入写作适配。"), {code: "writing_gateway_unavailable"});
  return browserFallback;
}

/** Checkpoint browser state remains in the writing plugin. Native Agent
 * preview uses the same statically bound checkpoint codec and transport. */
export function resolveWritingCheckpointRuntimeGateway(browserFallback: WritingCheckpointGateway): WritingCheckpointGateway {
  const runtime = getAppRuntime(), selected = runtime.gateways.getWritingCheckpointGateway;
  if (selected) return selected;
  if (runtime.mode === "preview" && !runtime.nativeAgentPreview)
    throw new Error("预览未覆盖业务网关：getWritingCheckpointGateway");
  if (runtime.mode === "desktop" || runtime.nativeAgentPreview)
    throw Object.assign(new Error("writing_checkpoint_gateway_unavailable：当前工作台尚未接入写作检查点适配。"), {code: "writing_checkpoint_gateway_unavailable"});
  return browserFallback;
}

/** Asset native access is desktop-only; preview assets retain their original
 * browser persistence path, even when Agent execution uses native preview. */
export function getWritingNoteAssetNativeGateway(): WritingNoteAssetGateway {
  const runtime = getAppRuntime(), selected = runtime.gateways.getWritingNoteAssetGateway;
  if (runtime.mode !== "desktop" || !selected)
    throw Object.assign(new Error("writing_note_asset_gateway_unavailable：当前桌面未接入写作笔记附件适配。"), {code: "writing_note_asset_gateway_unavailable"});
  return selected;
}
