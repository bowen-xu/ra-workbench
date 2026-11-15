import { resolveAppGateway } from "../runtime/appRuntime";
import { getAppStorage } from "../runtime/appStorage";
import { invoke, isDesktopRuntime } from "../runtime/platformCore";
import type {
  ContentGcRunReport,
  ContentStorageGateway,
  ContentStorageReport,
} from "@research-assistant/data";

const NOTE_ASSET_PREFIX = "research-assistant.note-asset.v1.";

function browserAssetTotals(): { readonly count: number; readonly bytes: number } {
  let count = 0;
  let bytes = 0;
  for (let index = 0; index < getAppStorage().length; index += 1) {
    const key = getAppStorage().key(index);
    if (key?.startsWith(NOTE_ASSET_PREFIX) !== true) continue;
    const raw = getAppStorage().getItem(key);
    if (raw === null) continue;
    try {
      const value = JSON.parse(raw) as { readonly bytes?: readonly number[] };
      count += 1;
      bytes += value.bytes?.length ?? 0;
    } catch {
      // Preserve unreadable preview data and exclude it from the estimate.
    }
  }
  return { count, bytes };
}

const browserGateway: ContentStorageGateway = {
  async getReport(): Promise<ContentStorageReport> {
    const totals = browserAssetTotals();
    const estimate = await navigator.storage?.estimate?.();
    const availableBytes = Math.max(0, (estimate?.quota ?? 0) - (estimate?.usage ?? 0));
    return {
      totalObjectCount: totals.count,
      totalBytes: totals.bytes,
      currentReferencedObjectCount: totals.count,
      currentReferencedBytes: totals.bytes,
      historyOnlyObjectCount: 0,
      historyOnlyBytes: 0,
      pinnedHistoryObjectCount: 0,
      pinnedHistoryBytes: 0,
      graceObjectCount: 0,
      graceBytes: 0,
      reclaimableObjectCount: 0,
      reclaimableBytes: 0,
      availableBytes,
      isLowDiskSpace: false,
      backfillStatus: "complete",
      gcStatus: { phase: "idle", isCancellable: false },
      lastSuccessCount: 0,
      lastSuccessBytes: 0,
    };
  },
  async getStatus() { return { phase: "idle", isCancellable: false }; },
  async scan() { return 0; },
  async clean(): Promise<ContentGcRunReport> {
    return { prunedRevisionCount: 0, markedObjectCount: 0, deletedObjectCount: 0, deletedBytes: 0, cleanupWarnings: [] };
  },
  async cancel() { return false; },
};

const desktopGateway: ContentStorageGateway = {
  getReport: () => invoke("get_content_storage_report"),
  getStatus: () => invoke("get_content_gc_status"),
  scan: () => invoke("scan_content_storage"),
  clean: (includeGracePeriod) => invoke("start_content_gc", { includeGracePeriod }),
  cancel: () => invoke("cancel_content_gc"),
};

export function getContentStorageGateway(): ContentStorageGateway {
  return resolveAppGateway("getContentStorageGateway", isDesktopRuntime() ? desktopGateway : browserGateway);
}
