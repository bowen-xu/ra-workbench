import type { AgentEvidenceImage, AgentEvidenceInput } from "@research-assistant/core/agent";
import { getNotesGateway } from "../notes/gateways";
import { hasPlatformCapabilities } from "../../runtime/appRuntime";
import { invoke } from "../../runtime/platformCore";
import { getAppStorage } from "../../runtime/appStorage";

// Undo owns image handles until its composer session ends or history is evicted.
const historyEvidence = new Map<object, readonly AgentEvidenceInput[]>();
export function retainHistoryEvidence(owner: object, evidence: readonly AgentEvidenceInput[]): void {
  const previous = historyEvidence.get(owner) ?? [];
  historyEvidence.set(owner, evidence.filter(item => item.image));
  releaseRemovedEvidence(previous);
}
export function releaseHistoryEvidence(owner: object): void {
  const previous = historyEvidence.get(owner); historyEvidence.delete(owner);
  releaseRemovedEvidence(previous);
}

/** Existing content-addressed asset import owns bytes; drafts carry metadata. */
export async function captureEvidenceImage(bytes: readonly number[], mediaType: string, title: string): Promise<AgentEvidenceImage> {
  if (!["image/png", "image/jpeg", "image/webp"].includes(mediaType) || !bytes.length || bytes.length > 524288) throw new Error("批注图片格式不支持或超过 512 KiB，请缩小选区。");
  const asset = await getNotesGateway().importNoteAsset({displayName:title,mimeType:mediaType,bytes});
  const request = {assetId:asset.id,mediaType:mediaType as AgentEvidenceImage["mediaType"],byteSize:bytes.length};
  return hasPlatformCapabilities() ? invoke<AgentEvidenceImage>("agent_pin_evidence_image",{request}) : {...request,captureId:crypto.randomUUID()};
}
export async function readEvidenceImage(image: AgentEvidenceImage): Promise<Uint8Array> {
  const bytes = hasPlatformCapabilities() ? Uint8Array.from(await invoke<number[]>("agent_read_evidence_image",{request:image})) : Uint8Array.from(await getNotesGateway().readNoteAsset(image.assetId));
  if (bytes.length !== image.byteSize) throw new Error("图片快照已损坏或缺失");
  return bytes;
}
export function releaseRemovedEvidence(before: readonly AgentEvidenceInput[] = [], after: readonly AgentEvidenceInput[] = []): void {
  if (!hasPlatformCapabilities()) return;
  const retained = new Set(after.flatMap(item=>item.type==="annotation"&&item.image ? [item.image.captureId] : []));
  for (const evidence of historyEvidence.values()) for (const item of evidence) if (item.image) retained.add(item.image.captureId);
  const persisted = persistedCaptureIds();
  if (!persisted) return; // Corrupt drafts must not lose retained images.
  for (const id of persisted) retained.add(id);
  const released = new Set<string>();
  for (const item of before) if(item.type==="annotation"&&item.image&&!retained.has(item.image.captureId)&&!released.has(item.image.captureId)) {released.add(item.image.captureId); void invoke("agent_release_evidence_image",{captureId:item.image.captureId}).catch(()=>{
    console.warn("图片草稿引用暂未清理，将在下次启动时重新核对。");
  });}
}
function persistedCaptureIds(): Set<string> | undefined {
  const storage=getAppStorage(), ids=new Set<string>();
  function visit(value:unknown):void {if(Array.isArray(value))value.forEach(visit);else if(value&&typeof value==="object"){const row=value as Record<string,unknown>;if(row.type==="annotation"&&row.image&&typeof row.image==="object"){const id=(row.image as Record<string,unknown>).captureId;if(typeof id==="string"&&/^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/u.test(id))ids.add(id);}Object.values(row).forEach(visit);}}
  try {for(let index=0;index<storage.length;index++){const key=storage.key(index);if(key&&(key==="research-assistant.agent-drafts.v1"||key.startsWith("research-assistant:independent-input:"))){const raw=storage.getItem(key);if(raw)visit(JSON.parse(raw));}}}catch{return undefined;}
  return ids;
}
export async function reconcileEvidenceDrafts():Promise<void> {
  if(!hasPlatformCapabilities())return;
  const ids = persistedCaptureIds();
  if (!ids) return;
  await invoke("agent_reconcile_evidence_drafts",{captureIds:[...ids]});
}
