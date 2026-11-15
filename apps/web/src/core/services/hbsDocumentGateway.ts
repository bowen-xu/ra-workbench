import { invoke } from "../runtime/platformCore";
import type { DocumentCommit } from "@research-assistant/core/hbs";
import { getAppStorage } from "../runtime/appStorage";
import { flushStorage } from "../runtime/storagePersistence";
import {getAppRuntime} from "../runtime/appRuntime";
import {createBrowserNoteHbsAuthority} from "../features/notes/gateways/browserNoteHbsAuthority";
import {NativeCommandDiagnostics} from "../runtime/NativeCommandDiagnostics";
import {diagnosticError} from "./diagnosticError";
import {hbsDocumentError, hbsNativeDiagnostic, hbsNativeRejectionIsDefinite} from "./hbsNativeFailure";

export type AppHbsDocumentKind = "note" | "structured";
/** App and full-app previews share this bounded authority contract. Decoding
 * stays in the typed DocumentPort/DocumentPreservationPort, never in a view. */
export interface AppHbsDocumentGateway {
  read(kind: AppHbsDocumentKind, resourceId: string): Promise<unknown>;
  commit(kind: AppHbsDocumentKind, resourceId: string, request: DocumentCommit<unknown>): Promise<unknown>;
  readPreservation(kind: AppHbsDocumentKind, resourceId: string): Promise<unknown>;
  savePreservation(kind: AppHbsDocumentKind, resourceId: string, checkpoint: unknown): Promise<unknown>;
}

const pendingKey = (kind: AppHbsDocumentKind, id: string) => `research-assistant:hbs-outbox:${kind}:${id}`;
const browserNotes = new WeakMap<Storage, ReturnType<typeof createBrowserNoteHbsAuthority>>();
// Metadata-only, bounded to the runtime storage owner's lifetime. No bodies,
// results, credentials or arbitrary exception strings enter this observation.
interface HbsObservation {
  readonly operations:NativeCommandDiagnostics;
  readonly failures:{command:string;kind:AppHbsDocumentKind;resourceId:string;requestId?:string;atMs:number;error:Record<string,unknown>}[];
  droppedFailures:number;
}
const observations = new WeakMap<Storage, HbsObservation>();
function call(command: string, kind: AppHbsDocumentKind, resourceId: string, value?: unknown): Promise<unknown> {
  const storage=getAppStorage();
  let observer=observations.get(storage);
  if(!observer){observer={operations:new NativeCommandDiagnostics(undefined,500),failures:[],droppedFailures:0};observations.set(storage,observer);}
  const owner=observer;
  return owner.operations.observe(`${command}:${kind}:${resourceId}`,async()=>{
    try{return await dispatch(command,kind,resourceId,value);}
    catch(reason){
      try {
        const id=value&&typeof value==="object"&&"id" in value?value.id:undefined;
        owner.failures.push({command,kind,resourceId,atMs:performance.now(),error:{...diagnosticError(reason),...hbsNativeDiagnostic(reason)},
          ...(typeof id==="string"&&/^[\w:.-]{1,200}$/.test(id)?{requestId:id}:{})});
        if(owner.failures.length>100){owner.failures.shift();owner.droppedFailures++;}
      } catch { /* Diagnostic capture cannot replace the original failure. */ }
      throw reason;
    }
  });
}
export function readHbsDocumentDiagnostics(kind:AppHbsDocumentKind,resourceId:string) {
  const owner=observations.get(getAppStorage()),snapshot=owner?.operations.read();
  return {timeOrigin:performance.timeOrigin,dropped:snapshot?.dropped??0,
    droppedFailures:owner?.droppedFailures??0,failures:owner?.failures.filter(entry=>entry.kind===kind&&entry.resourceId===resourceId)??[],
    entries:snapshot?.entries.filter(entry=>entry.command.endsWith(`:${kind}:${resourceId}`))??[]};
}
function dispatch(command: string, kind: AppHbsDocumentKind, resourceId: string, value?: unknown): Promise<unknown> {
  if (getAppRuntime().mode === "browser") {
    if (kind !== "note") return Promise.reject(new Error("MODEL_PLATFORM_UNAVAILABLE：结构化资源需要桌面应用。"));
    const storage = getAppStorage();
    let authority = browserNotes.get(storage);
    if (!authority) { authority = createBrowserNoteHbsAuthority(); browserNotes.set(storage, authority); }
    if (command === "hbs_read_document") return authority.read(resourceId);
    if (command === "hbs_commit_document") return authority.commit(resourceId, value as DocumentCommit<unknown>);
    if (command === "hbs_read_preservation") return authority.readPreservation(resourceId);
    return authority.savePreservation(resourceId, value);
  }
  return invoke(command, {kind, resourceId, ...(command === "hbs_commit_document" ? {request: value}
    : command === "hbs_save_preservation" ? {checkpoint: value} : {})}).catch(reason => {
      // Tauri serializes command errors as objects. Keep the safe diagnostic
      // fields so the framework-independent session does not print [object Object].
      throw hbsDocumentError(reason);
    });
}
export function getPendingHbsDocumentCommit(kind: AppHbsDocumentKind, id: string): DocumentCommit<unknown> | undefined {
  const raw = getAppStorage().getItem(pendingKey(kind, id));
  if (!raw) return;
  const value: unknown = JSON.parse(raw);
  if (!value || typeof value !== "object" || !("id" in value) || typeof value.id !== "string"
    || !("expectedVersion" in value) || !Number.isSafeInteger(value.expectedVersion)
    || !("saved" in value) || !("baseline" in value)) throw new Error("MODEL_RECOVERY_INVALID：待核对请求损坏，原记录已保留。");
  return value as DocumentCommit<unknown>;
}
function definiteReply(value: unknown, requestId: string): boolean {
  if (!value || typeof value !== "object") return false;
  const reply = value as { type?: unknown; requestId?: unknown; snapshot?: unknown };
  return ["receipt", "conflict"].includes(String(reply.type)) && reply.requestId === requestId
    && !!reply.snapshot && typeof reply.snapshot === "object" && "version" in reply.snapshot
    && Number.isSafeInteger(reply.snapshot.version) && Number(reply.snapshot.version) >= 0 && "saved" in reply.snapshot && "baseline" in reply.snapshot;
}

/** Alternate first-publication transports use the same immutable outbox and
 * definite-reply cleanup as subsequent HBS saves. */
export async function commitHbsDocumentRequest(kind: AppHbsDocumentKind, resourceId: string, request: DocumentCommit<unknown>, transport: () => Promise<unknown>): Promise<unknown> {
    const storage = getAppStorage(), key = pendingKey(kind, resourceId), previous = getPendingHbsDocumentCommit(kind, resourceId);
    if (previous && JSON.stringify(previous) !== JSON.stringify(request)) {
      throw Object.assign(new Error(`MODEL_REQUEST_UNSETTLED：请求 ${previous.id} 尚未核对，请先重试原请求。`), { code: "MODEL_REQUEST_UNSETTLED", requestId: previous.id });
    }
    storage.setItem(key, JSON.stringify(request)); await flushStorage(storage);
    if (getAppStorage() !== storage) throw Object.assign(new Error("MODEL_OWNER_CHANGED：存储拥有者已切换，原保存请求尚未发送。"), {code: "MODEL_OWNER_CHANGED", requestId: request.id});
    try {
      const reply: unknown = await transport();
      if (definiteReply(reply, request.id)) { storage.removeItem(key); await flushStorage(storage); }
      return reply;
    } catch (reason) {
      const code = reason && typeof reason === "object" && "code" in reason ? String(reason.code) : "";
      if (hbsNativeRejectionIsDefinite(reason,request.id) !== false && ["MODEL_INVALID_REQUEST", "MODEL_INVALID_DOCUMENT", "MODEL_INVALID_BASELINE", "MODEL_DRAFT_PENDING", "MODEL_NOT_FOUND",
        "MODEL_RESOURCE_INACTIVE", "MODEL_LEGACY_IMPORT_REQUIRED", "MODEL_LEGACY_IMPORT_CONFLICT", "MODEL_REQUEST_TOO_LARGE",
        "MODEL_INVALID_KIND", "MODEL_INVALID_RESOURCE", "MODEL_SOURCE_SAVE_FAILED", "invalid_input", "not_found", "revision_conflict"].includes(code)) {
        storage.removeItem(key); await flushStorage(storage);
      }
      throw reason;
    }
}
const gateway: AppHbsDocumentGateway = {
  read: (kind, resourceId) => call("hbs_read_document", kind, resourceId),
  commit: (kind, resourceId, request) => commitHbsDocumentRequest(kind,resourceId,request,()=>call("hbs_commit_document",kind,resourceId,request)),
  readPreservation: (kind, resourceId) => call("hbs_read_preservation", kind, resourceId),
  savePreservation: (kind, resourceId, checkpoint) => call("hbs_save_preservation", kind, resourceId, checkpoint),
};

export function getHbsDocumentGateway(): AppHbsDocumentGateway { return gateway; }
/** Explicit retry only. Opening a document never resends an uncertain write. */
export async function retryPendingHbsDocumentCommit(kind: AppHbsDocumentKind, id: string): Promise<unknown> {
  const request = getPendingHbsDocumentCommit(kind, id);
  return request ? gateway.commit(kind, id, request) : undefined;
}
