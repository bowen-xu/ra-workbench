import type {AgentEvent} from "@research-assistant/core/agent";
import {noteBlockId} from "@research-assistant/core/notes";
import {getAppRuntime} from "../../../core/runtime/appRuntime";
import {flushStorage} from "../../../core/runtime/storagePersistence";
import {getNotesGateway} from "../../../core/features/notes/gateways";
import {writingDemoContextCatalog} from "./WritingDemoContextCatalog";

/** Data and lifecycle fixtures for browser tests. The normal App never imports
 * this module, and no method grants desktop access or returns a gateway. */
function browserRuntime() {
  const runtime = getAppRuntime();
  if (runtime.mode === "desktop") throw new Error("BROWSER_FIXTURE_DESKTOP_FORBIDDEN");
  return runtime;
}

function agentPorts() {
  const runtime = browserRuntime();
  const controls = runtime.preview;
  const context = runtime.gateways.agentContextGateway;
  const sessions = runtime.gateways.agentSessionGateway;
  const metadata = runtime.gateways.agentMetadataGateway;
  if (runtime.mode !== "preview" || !controls || !context || !sessions || !metadata) {
    throw new Error("BROWSER_FIXTURE_PREVIEW_REQUIRED");
  }
  return {runtime, controls, context, sessions, metadata};
}

export function configureAgentPreview(values: {delayMs: number; nextFailure: string | null}): void {
  agentPorts().controls.configure(values);
}

export function configureLiveAgentResponse(replacement?: {text: string; process?: boolean}): void {
  const {controls, context, sessions} = agentPorts();
  controls.configure({delayMs: 0, nextFailure: null});
  if (!replacement) return;
  for (const gateway of new Set([sessions, context.sessions])) {
    const assistantItems = new Set<string>();
    const replacedItems = new Map<string, number>();
    const read = gateway.readHistory.bind(gateway);
    gateway.readHistory = async (...args: Parameters<typeof read>) => {
      const result = await read(...args);
      return {...result, events: result.events.map((event): AgentEvent => {
        const payload = event.payload;
        if (payload.type === "itemStarted" && payload.kind.type === "assistantMessage") {
          assistantItems.add(payload.itemId);
        }
        if (payload.type === "textDelta" && assistantItems.has(payload.itemId)) {
          const firstSeq = replacedItems.get(payload.itemId) ?? event.seq;
          replacedItems.set(payload.itemId, firstSeq);
          return {...event, payload: {...payload, text: firstSeq === event.seq ? replacement.text : ""}};
        }
        if (replacement.process && payload.type === "itemPhaseChanged" && assistantItems.has(payload.itemId)) {
          return {...event, payload: {...payload, phase: "commentary"}};
        }
        return event;
      })};
    };
  }
}

export function holdLiveAgentOutput(delayMs = 60_000): void {
  if (!Number.isFinite(delayMs) || delayMs < 100 || delayMs > 60_000) {
    throw new Error("BROWSER_FIXTURE_OUTPUT_DELAY_INVALID");
  }
  const {controls} = agentPorts();
  controls.configure({delayMs: 0, nextFailure: null});
  Object.defineProperty(controls, "delayMs", {get: () => delayMs});
  controls.beforeOperation = async () => undefined;
}

export async function liveAgentTurnIds(): Promise<string[]> {
  const {runtime, sessions} = agentPorts();
  const conversationId = runtime.storage.getItem("research-assistant:independent-selected");
  if (!conversationId) return [];
  const result = await sessions.readHistory({conversationId, afterSeq: 0, limit: 512});
  return result.events.flatMap(event => event.payload.type === "turnFinished" ? [event.payload.turnId] : []);
}

export function configureFailedAgentHistory(): void {
  const {controls, context, sessions} = agentPorts();
  controls.configure({delayMs: 0, nextFailure: null});
  for (const gateway of new Set([sessions, context.sessions])) {
    const read = gateway.readHistory.bind(gateway);
    gateway.readHistory = async (...args: Parameters<typeof read>) => {
      const result = await read(...args);
      return {...result, events: result.events.map((event): AgentEvent =>
        event.payload.type === "turnFinished" && event.payload.outcome === "completed"
          ? {...event, payload: {...event.payload, outcome: "failed"}} : event)};
    };
  }
}

export function installLiveContextCatalog(): void {
  const rows = writingDemoContextCatalog("00000000-0000-4000-8000-000000000003", ["main.tex"]);
  agentPorts().context.catalog = async query => rows.filter(row => row.label.includes(query));
}

let releaseMetadata: (() => void) | undefined;
let metadataBlocked = false;
export function holdAgentMetadata(failed: boolean): void {
  const {metadata} = agentPorts();
  metadataBlocked = false;
  const read = metadata.read.bind(metadata);
  let release!: () => void;
  const pending = new Promise<void>(resolve => {release = resolve;});
  let armed = true;
  metadata.read = async (...args: Parameters<typeof read>) => {
    if (armed) {
      metadataBlocked = true;
      await pending;
      armed = false;
      if (failed) throw new Error("synthetic metadata unavailable");
    }
    return read(...args);
  };
  releaseMetadata = release;
}

export function agentMetadataBlocked(): boolean {
  agentPorts();
  return metadataBlocked;
}

export function releaseAgentMetadata(): void {
  if (!releaseMetadata) throw new Error("BROWSER_FIXTURE_METADATA_NOT_HELD");
  releaseMetadata();
  releaseMetadata = undefined;
}

export function installAgentStartFailure(lostReceipt: boolean): void {
  const start = agentPorts().context.guardedStart;
  if (!start) throw new Error("BROWSER_FIXTURE_GUARDED_START_REQUIRED");
  const original = start.start.bind(start);
  const read = start.read.bind(start);
  const starts: string[] = [], statusReads: string[] = [];
  sessionStorage.removeItem("test-start-requests");
  sessionStorage.removeItem("test-start-status-requests");
  start.read = async (...args: Parameters<typeof read>) => {
    statusReads.push(args[0].requestId);
    sessionStorage.setItem("test-start-status-requests", JSON.stringify(statusReads));
    return read(...args);
  };
  let failed = false;
  start.start = async (...args: Parameters<typeof original>) => {
    starts.push(args[0].requestId);
    sessionStorage.setItem("test-start-requests", JSON.stringify(starts));
    if (!failed) {
      failed = true;
      if (lostReceipt) {await original(...args); throw new Error("lost receipt");}
      throw "modelUnavailable";
    }
    return original(...args);
  };
}

export async function createPreviewNote(title: string): Promise<string> {
  const runtime = browserRuntime();
  const result = await getNotesGateway().createPage({title});
  await flushStorage(runtime.storage);
  return result.page.id;
}

export async function createBrowserLinkNote(): Promise<string> {
  const runtime = browserRuntime(), gateway = getNotesGateway();
  const initial = await gateway.createPage({title: "Browser adapter sample"});
  await gateway.saveDocument({expectedRevision: initial.page.revision, saveReason: "structure",
    document: {...initial.document, blocks: [{id: noteBlockId(crypto.randomUUID()), type: "paragraph", props: {}, children: [],
      content: {kind: "inline", items: [
        {type: "link", href: "https://example.org/research", content: [{type: "text", text: "External web", styles: []}]},
        {type: "text", text: " ", styles: []},
        {type: "link", href: `source://notes/${initial.page.id}`, content: [{type: "text", text: "Internal note", styles: []}]},
      ]}}]}});
  await flushStorage(runtime.storage);
  return initial.page.id;
}

export function configureBrowserOpenFailure(): void {
  Object.assign(browserRuntime(), {browser: {open: async () => {throw new Error("Synthetic OS error 5");}}});
}

const browserUrls: string[] = [];
export function recordBrowserOpenings(): void {
  browserUrls.length = 0;
  Object.assign(browserRuntime(), {browser: {open: async (url: string) => {browserUrls.push(url);}}});
}

export function readBrowserOpenings(): string[] {return [...browserUrls];}

/** Bounded synthetic submission observations from the original mock owner. */
export function readIndependentSubmissionTexts(): readonly string[] {
  const {runtime} = agentPorts();
  const rows = JSON.parse(runtime.storage.getItem("independent-submissions") ?? "[]") as
    readonly (readonly [string, {readonly input: {readonly text: string}}])[];
  return rows.slice(-16).map(([, row]) => row.input.text);
}
