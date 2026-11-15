import { useSyncExternalStore } from "react";
import type { NoteEditorInstallPort } from "./noteEditorInstallPort";
import type { NoteDocumentV1 } from "@research-assistant/core/notes";
import type { NoteSaveStatus } from "./noteSaveController";
export interface AppliedNotePage {
  readonly pageId: string; readonly revision: number; readonly document: NoteDocumentV1;
  readonly review?: { readonly baseline: NoteDocumentV1; readonly origin: "agent" | "review" | "restore" | "manual";
    readonly requestId?: string | null; readonly addToHistory: boolean };
}
const receiptListeners = new Set<(pages: readonly AppliedNotePage[]) => Promise<void>>();
export interface NoteReviewSaveState { readonly pendingChanges?: number; readonly status: NoteSaveStatus; readonly message?: string }
export interface NoteReviewMenuAction {
  readonly id:string;readonly label:string;readonly group:string;
  readonly icon:"keep"|"retry"|"open";readonly disabled:boolean;
  readonly run:()=>void;
}
interface NoteReviewSavePort {
  flush(): Promise<void>;
  retry(): Promise<void>;
  getState(): NoteReviewSaveState;
  synchronize?(): Promise<void>;
  recovery?(): import("./noteReviewRecovery").NoteReviewRecovery | undefined;
  discardRecovery?(): void;
  inspect?(): import("@research-assistant/core/hbs").HbsSessionState<NoteDocumentV1>;
  acknowledgeReview?(requestId:string):void;
}
/** Non-body operations can contribute their existing status and exact retry.
 * They cannot enqueue or persist an HBS document through this interface. */
export interface NoteReviewStateContributor {
  readonly source: "metadata";
  getState(): NoteReviewSaveState;
  retry(): Promise<void>;
  recovery?(): import("./noteReviewRecovery").NoteReviewRecovery | undefined;
  menuActions?():readonly NoteReviewMenuAction[];
}
const savedReview: NoteReviewSaveState = { status: "saved" };
const reviewSaves = new Map<string, NoteReviewSavePort>();
const contributions = new Map<string, Set<NoteReviewStateContributor>>();
const aggregates = new Map<string, { readonly states: readonly NoteReviewSaveState[]; readonly value: NoteReviewSaveState }>();
const emptyMenuActions:readonly NoteReviewMenuAction[]=[];
const menuAggregates=new Map<string,{readonly sources:readonly (readonly NoteReviewMenuAction[])[];readonly value:readonly NoteReviewMenuAction[]}>();
const savePriority = { saved: 0, dirty: 1, saving: 2, conflict: 3, failed: 4 };
const reviewSaveListeners = new Set<() => void>();
/** Exposes the existing review writer's progress, flush and exact-request
 * recovery to its save host. This port owns no queue or persistence. */
export function registerNoteReviewSavePort(page: string, port: NoteReviewSavePort): () => void {
  reviewSaves.set(page, port); notifyNoteReviewSaveState();
  return () => { if (reviewSaves.get(page) === port) { reviewSaves.delete(page); notifyNoteReviewSaveState(); } };
}
export function notifyNoteReviewSaveState(): void { reviewSaveListeners.forEach(listener => listener()); }
/** A verified non-body receipt releases only its matching recovered request.
 * The original document owner keeps all newer H/S′ and its own save queue. */
export function acknowledgeNoteReviewRequest(page:string,requestId:string):void {reviewSaves.get(page)?.acknowledgeReview?.(requestId);}
export function registerNoteReviewStateContributor(page: string, contributor: NoteReviewStateContributor): () => void {
  const group = contributions.get(page) ?? new Set(); group.add(contributor); contributions.set(page, group); notifyNoteReviewSaveState();
  return () => { group.delete(contributor); if (!group.size) contributions.delete(page); aggregates.delete(page); menuAggregates.delete(page); notifyNoteReviewSaveState(); };
}
export function getNoteReviewSaveState(page: string): NoteReviewSaveState {
  const primary = reviewSaves.get(page)?.getState() ?? savedReview, extra = [...(contributions.get(page) ?? [])].map(port => port.getState());
  if (!extra.length) return primary;
  const states = [primary, ...extra], cached = aggregates.get(page);
  if (cached?.states.length === states.length && states.every((state, index) => state === cached.states[index])) return cached.value;
  const strongest = states.reduce((selected, state) => savePriority[state.status] > savePriority[selected.status] ? state : selected);
  const messages = states.flatMap((state, index) => state.message ? [`${index ? "页面及关联" : "正文"}：${state.message}`] : []);
  const value: NoteReviewSaveState = { status: strongest.status, pendingChanges: states.reduce((count, state) => count + (state.pendingChanges ?? 0), 0),
    ...(messages.length ? { message: messages.join("；") } : {}) };
  aggregates.set(page, { states, value }); return value;
}
export function useNoteReviewSaveState(page: string | undefined): NoteReviewSaveState {
  return useSyncExternalStore(listener => { reviewSaveListeners.add(listener); return () => reviewSaveListeners.delete(listener); },
    () => page ? getNoteReviewSaveState(page) : savedReview, () => savedReview);
}
/** Recovery commands stay with the existing page owner and status menu.
 * Snapshot identity must be stable between actual contribution updates. */
export function useNoteReviewMenuActions(page:string|undefined):readonly NoteReviewMenuAction[] {
  return useSyncExternalStore(listener=>{reviewSaveListeners.add(listener);return()=>reviewSaveListeners.delete(listener);},()=>{
    if(!page)return emptyMenuActions;
    const sources=[...(contributions.get(page)??[])].map(port=>port.menuActions?.()??emptyMenuActions);
    if(!sources.length)return emptyMenuActions;
    const previous=menuAggregates.get(page);
    if(previous&&sources.length===previous.sources.length&&sources.every((source,index)=>source===previous.sources[index]))return previous.value;
    const value=sources.flat();menuAggregates.set(page,{sources,value});return value;
  },()=>emptyMenuActions);
}
export async function flushNoteReview(page: string): Promise<void> {
  for (const port of contributions.get(page) ?? []) if (port.getState().status !== "saved") throw new Error(port.getState().message ?? "MODEL_METADATA_UNSETTLED：页面及关联修改尚未确认。");
  await reviewSaves.get(page)?.flush();
}
export async function retryNoteReview(page: string): Promise<void> {
  const primary = reviewSaves.get(page), extra = [...(contributions.get(page) ?? [])].filter(port => port.getState().status === "failed" || port.getState().status === "conflict");
  if (!extra.length || primary?.getState().status !== "saved") await primary?.retry();
  for (const port of extra) await port.retry();
}
export async function synchronizeNoteReview(page: string): Promise<void> { await reviewSaves.get(page)?.synchronize?.(); }
export function noteReviewRecovery(page: string): import("./noteReviewRecovery").NoteReviewRecovery | undefined {
  const extra = [...(contributions.get(page) ?? [])].map(port => port.recovery?.()).find(recovery => recovery?.request);
  return extra ?? reviewSaves.get(page)?.recovery?.();
}
export function discardNoteReviewRecovery(page: string): void { reviewSaves.get(page)?.discardRecovery?.(); }
export function noteEditorDocument(page: string): NoteDocumentV1 | undefined {
  const group = ports.get(page); return group?.size === 1 ? group.values().next().value?.read() : undefined;
}
/** Read-only export of the existing owner; never flushes or retries a request. */
export function inspectNoteHbsSession(page: string): import("@research-assistant/core/hbs").HbsSessionState<NoteDocumentV1> | undefined {
  return reviewSaves.get(page)?.inspect?.();
}
export function registerTextReceiptListener(listener: (pages: readonly AppliedNotePage[]) => Promise<void>): () => void {
  receiptListeners.add(listener); return () => receiptListeners.delete(listener);
}
export async function deliverTextReceipt(pages: readonly AppliedNotePage[]): Promise<void> { await Promise.all([...receiptListeners].map(listener => listener(pages))); }
const ports = new Map<string, Set<NoteEditorInstallPort>>();
const snapshotChecks = new Map<NoteEditorInstallPort, () => boolean>();
const held = new Map<string, Set<string>>();
// Metadata preparation freezes user input but lets the current save owner flush
// S/S′. Other hold tokens represent external writers and block that preparation.
const inputGuards = new Set<string>();
const listeners = new Set<() => void>();
export function registerTextInstallPort(page: string, port: NoteEditorInstallPort, canSnapshot: () => boolean = () => true): () => void {
  const group = ports.get(page) ?? new Set(); group.add(port); ports.set(page, group);
  snapshotChecks.set(port, canSnapshot);
  for (const request of inputGuards) holdTextEditor(page, request, true);
  if (held.get(page)?.size) port.setWriteLocked?.(true);
  return () => { snapshotChecks.delete(port); group.delete(port); if (!group.size) ports.delete(page); };
}
export function checkTextSnapshotReady(page: string): void {
  if (!snapshotChecks.get(textInstallPort(page))?.()) throw new Error("编辑器正在输入、导入附件或核对审阅请求，请完成后重试。");
}
export function textInstallPort(page: string): NoteEditorInstallPort {
  const port = availableTextInstallPort(page);
  if (!port) throw new Error("笔记编辑器未就绪，请保留输入后重试。");
  return port;
}
/** A retained session may reconcile before its view has mounted. Duplicate
 * installations are still an ownership failure, never treated as no view. */
export function availableTextInstallPort(page: string): NoteEditorInstallPort | undefined {
  const group = ports.get(page);
  if (!group?.size) return undefined;
  if (group.size !== 1) throw new Error("笔记存在多个编辑会话，请保留输入后重试。");
  return group.values().next().value!;
}
export function holdTextEditor(page: string, request: string, value: boolean): void {
  const requests = held.get(page) ?? new Set();
  if (value) requests.add(request); else requests.delete(request);
  if (requests.size) held.set(page, requests); else held.delete(page);
  ports.get(page)?.forEach(port => port.setWriteLocked?.(requests.size > 0));
  listeners.forEach(listener => listener());
}
/** Freeze current and newly mounted editors until a metadata transaction ends.
 * Preflight input composition/import before taking any locks. This grants no
 * external write lease and never bypasses another writer's hold. */
export function holdNoteEditorInput(request: string): () => void {
  for (const page of ports.keys()) checkTextSnapshotReady(page);
  inputGuards.add(request);
  for (const page of ports.keys()) holdTextEditor(page, request, true);
  return () => {
    inputGuards.delete(request);
    for (const page of held.keys()) if (held.get(page)?.has(request)) holdTextEditor(page, request, false);
  };
}
export function isTextEditorSaveBlocked(page: string): boolean {
  return [...(held.get(page) ?? [])].some(request => !inputGuards.has(request));
}
export function useTextEditorHeld(page: string): boolean {
  return useSyncExternalStore(listener => { listeners.add(listener); return () => listeners.delete(listener); },
    () => (held.get(page)?.size ?? 0) > 0, () => false);
}
export function isTextEditorHeld(page: string): boolean { return (held.get(page)?.size ?? 0) > 0; }
