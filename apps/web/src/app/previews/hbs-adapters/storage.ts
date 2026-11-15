import { createDocumentPort, createDocumentPreservationPort, decodeDocumentSnapshot } from "@research-assistant/data/hbs";
import { HbsSession, type HbsDocumentAdapter } from "@research-assistant/core/hbs";
import { HbsDeliveryPort } from "../../../core/components/hbs/deliveryPort";
import { createIndexedHbsAuthority } from "./indexedAuthority";
import { readFixtureAsset, fixtureAssets } from "./fixtureAssets";

type Authority = Awaited<ReturnType<typeof createIndexedHbsAuthority>>;
const authorities = new Map<string, Promise<Authority>>();
let pending = 0, guarded = false;
function protectPendingWrites(): void {
  if (guarded) return;
  guarded = true;
  window.addEventListener("beforeunload", event => { if (pending) { event.preventDefault(); event.returnValue = ""; } });
  // Connections belong to this page (at most one per kind). The browser closes
  // them on unload; do not close ahead of the session's queued preservation.
}

/** This is a browser persistence boundary, not an HTTP or desktop bridge. */
export async function previewCall(kind: string, operation: string, resourceId?: string, request?: unknown): Promise<unknown> {
  if (kind === "note" && operation === "fixtureAssets") return fixtureAssets;
  if (kind === "note" && operation === "readAsset") return readFixtureAsset(resourceId ?? "");
  const authority = authorities.get(kind);
  if (!authority) throw new Error("MODEL_STORAGE_UNAVAILABLE：请先打开对应的示例资源。");
  pending++;
  try {
    const store = await authority;
    if (resourceId !== undefined && store.id !== resourceId) throw new Error("MODEL_IDENTITY_MISMATCH：预览资源身份不匹配。");
    return await store.call(operation, request);
  } finally { pending--; }
}

export async function openExample<D>(adapter: HbsDocumentAdapter<D>, fixture: D) {
  protectPendingWrites();
  let authority = authorities.get(adapter.kind);
  if (!authority) {
    authority = createIndexedHbsAuthority(adapter, fixture);
    authorities.set(adapter.kind, authority);
    void authority.catch(() => { if (authorities.get(adapter.kind) === authority) authorities.delete(adapter.kind); });
  }
  const store = await authority, id = store.id;
  const actual = createDocumentPort(adapter, (operation, request) => previewCall(adapter.kind, operation, id, request));
  const delivery = new HbsDeliveryPort(actual, "IndexedDB");
  const preservation = createDocumentPreservationPort(adapter, (operation, request) => previewCall(adapter.kind, operation, id, request));
  const prefix = crypto.randomUUID();
  const session = await HbsSession.open({ adapter, port: delivery, preservation, requestId: n => `${prefix}:${n}` });
  const inspect = async () => {
    const value = await previewCall(adapter.kind, "inspectStorage", id) as {
      snapshot: unknown; storage: Record<"S" | "B" | "S′", readonly import("../../../core/components/hbs/HbsStorageRows").HbsStoredTable[]>;
    };
    return { ...decodeDocumentSnapshot(value.snapshot, adapter), storage: value.storage };
  };
  return { id, actual, delivery, session, inspect };
}
