import "./appPreviewDatabase";
import {configureBuiltinServices} from "./configureBuiltinServices";
import { isTauri } from "@tauri-apps/api/core";
import { PREVIEW_STORAGE_PREFIX, selectAppStorage } from "../../core/runtime/appStorage";
import { createIndexedPreviewStorage } from "../../core/runtime/previewIndexedStorage";
import { withPreviewPersistence } from "../../core/runtime/previewPersistenceRuntime";
import { flushStorage } from "../../core/runtime/storagePersistence";
import { selectAppRuntime, type AppRuntime } from "../../core/runtime/appRuntime";
import { isSourceNavigationPreview } from "../../core/runtime/sourceNavigationPreview";
import { readWritingRecoveryScenario } from "../../core/runtime/writingRecoveryPreview";
import {readNoteRecoveryScenario} from "../../core/runtime/noteRecoveryPreview";
import { readNoteOpeningScenario, mockNoteOpening } from "./mock/mockNoteOpening";

/** Both full-App preview URLs select the same isolated mock environment. */
export function isAppPreviewRequest(search: string): boolean {
  const params = new URLSearchParams(search);
  return params.get("preview") === "notes" || params.get("preview") === "app" || params.get("demo") === "app" || params.get("demo") === "text-patch";
}

export async function createAppRuntime(search: string, backing: Storage): Promise<AppRuntime> {
  if (isAppPreviewRequest(search)) {
    const sourcePreview = isSourceNavigationPreview(search);
    const params = new URLSearchParams(search);
    const textPatch = params.get("demo") === "text-patch" || params.get("preview") === "notes";
    const bibliographyPreview = new URLSearchParams(search).get("bibliography") === "1";
    const writingRecovery = readWritingRecoveryScenario(search);
    const noteRecovery = readNoteRecoveryScenario(search);
    const noteOpening = readNoteOpeningScenario(search);
    const prefix = noteOpening ? `research-assistant.note-opening-preview.v1:${noteOpening}:` : noteRecovery ? `research-assistant.note-recovery-preview.v1:${noteRecovery}:` : bibliographyPreview ? "research-assistant.bibliography-preview.v3:" : writingRecovery ? `research-assistant.writing-recovery-preview.v3:${writingRecovery}:`
      : sourcePreview ? "research-assistant.source-preview.v3:"
      : textPatch ? "research-assistant.text-patch-preview.v5:" : PREVIEW_STORAGE_PREFIX;
    const storage = await createIndexedPreviewStorage(prefix);
    selectAppStorage(storage);
    selectAppRuntime({ mode: "browser", storage, gateways: {}, dispose() {} });
    const { createMockAppRuntime } = await import("./mock/createMockAppRuntime");
    const runtime = withPreviewPersistence(await createMockAppRuntime(storage, { sourceNavigation: sourcePreview, textPatch, bibliographyPreview, ...(writingRecovery ? { writingRecovery } : {}), ...(noteRecovery ? {noteRecovery} : {}) }));
    if (noteOpening && runtime.gateways.getNotesGateway) {
      const { TEXT_PATCH_PREVIEW_PAGE } = await import("./mock/mockTextPatchGateway");
      runtime.gateways.getNotesGateway = mockNoteOpening(runtime.gateways.getNotesGateway, noteOpening, TEXT_PATCH_PREVIEW_PAGE);
    }
    await flushStorage(storage);
    selectAppRuntime(runtime);
    return runtime;
  }
  const runtime: AppRuntime = { mode: isTauri() ? "desktop" : "browser", storage: backing, gateways: {}, dispose() {} };
  selectAppRuntime(runtime);
  configureBuiltinServices(runtime);
  return runtime;
}
