import type {HbsSessionState} from "@research-assistant/core/hbs";
import type {NoteDocumentV1} from "@research-assistant/core/notes";

import {diagnosticError} from "../../../services/diagnosticError";
export {diagnosticError} from "../../../services/diagnosticError";

export async function documentFingerprint(value: unknown): Promise<{sha256?: string; bytes: number}> {
  const bytes = new TextEncoder().encode(JSON.stringify(value) ?? "null");
  if (!globalThis.crypto?.subtle) return {bytes:bytes.byteLength};
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return {sha256:[...new Uint8Array(digest)].map(n=>n.toString(16).padStart(2,"0")).join(""),bytes:bytes.byteLength};
}
export function sessionDiagnostics(state: HbsSessionState<NoteDocumentV1> | undefined) {
  if (!state) return {available:false};
  return {available:true,version:state.observed.version,pending:state.pending,queued:state.queued,
    changes:state.changes.map(change=>({id:change.id,kind:change.kind})),
    conflicts:state.conflicts.map(change=>({id:change.id,kind:change.kind})),
    preservationError:state.preservationError ? diagnosticError(state.preservationError) : undefined,
    events:state.events.map((message,index)=>({sequence:index+1,...diagnosticError(message),
      requestId:message.match(/请求\s+([\w:.-]{1,200})/u)?.[1],
      versions:[...message.matchAll(/\bv(\d+)\b/g)].map(match=>Number(match[1]))})),
    eventsHaveTimestamps:false};
}
