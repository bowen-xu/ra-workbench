/** @vitest-environment happy-dom */
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import {agentQueueEditIntentFromRequest, type AgentQueueEdit, type AgentQueueState} from "@research-assistant/core/agent";
import type { AgentInputQueueState } from "@research-assistant/data/agent";
import { AgentQueueProvider, useAgentQueueOwners } from "./AgentQueueProvider";
import { AgentDraftProvider, useAgentDraftStore, useConversationDraft } from "./AgentDraftProvider";
import { useWritingInputQueue } from "../../../../../../plugins/writing_projects/src/features/writing/agent/useWritingInputQueue";
import { useNoteInputQueue } from "../notes/agent/useNoteInputQueue";
import {useConversationInputQueue} from "./context/useConversationInputQueue";
import {agentContextGateway} from "../../services/agentContextGateway";
const mocks = vi.hoisted(() => ({ writing: { read: vi.fn(), edit: vi.fn() }, notes: { read: vi.fn(), edit: vi.fn() }, papers: { read: vi.fn(), edit: vi.fn() }, conversations:{read:vi.fn(),edit:vi.fn(),inspectEdit:vi.fn()} }));
vi.mock("../../services/agentGateway", () => ({ agentInputQueueGateway: mocks.writing, agentNoteQueueGateway: mocks.notes, agentPaperQueueGateway: mocks.papers, agentTurnGateway: {} }));
vi.mock("../../services/agentContextGateway",()=>({agentContextGateway:{queue:mocks.conversations}}));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
type Kind = "manuscript" | "note" | "readingNote";
const empty: AgentQueueState = { schemaVersion: 1, conversationId: "11111111-1111-4111-8111-111111111111", revision: 0, paused: false, preparingInputId: null, inputs: [] };

it("retains an independent conversation's pending queue write and acknowledgment after its view unmounts",async()=>{
  mocks.conversations.read.mockResolvedValue(empty);
  let finish!: (value:AgentQueueState)=>void;
  mocks.conversations.edit.mockImplementation(()=>new Promise(resolve=>{finish=resolve;}));
  let queue!:ReturnType<typeof useConversationInputQueue>;
  function Conversation(){queue=useConversationInputQueue(agentContextGateway,empty.conversationId,true,0);return null;}
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  const render=async(show:boolean)=>{await act(async()=>root.render(<StrictMode><AgentQueueProvider>{show&&<Conversation/>}</AgentQueueProvider></StrictMode>));};
  const acknowledge=vi.fn();let pending!:Promise<boolean>;
  try {
    await render(true);
    await act(async()=>{pending=queue.controller.current!.edit({type:"add",inputId:crypto.randomUUID(),text:"Queued"},acknowledge);});
    await render(false);
    await act(async()=>{finish({...empty,revision:1});await pending;});
    expect(acknowledge).toHaveBeenCalledOnce();
    expect(mocks.conversations.edit).toHaveBeenCalledOnce();
  } finally {await act(async()=>root.unmount());host.remove();}
});

it("inspects the original independent resume receipt through the shared owner before replaying", async () => {
  const request = {conversationId: empty.conversationId, requestId: crypto.randomUUID(), expectedRevision: 0,
    expectedEpoch: crypto.randomUUID(), edit: {type: "resume" as const}};
  const saved = {...empty, revision: 1};
  mocks.conversations.read.mockReset().mockResolvedValue(saved);
  mocks.conversations.edit.mockReset().mockRejectedValue(new Error("must not replay"));
  mocks.conversations.inspectEdit.mockReset().mockResolvedValue({schemaVersion: 1, intent: agentQueueEditIntentFromRequest(request),
    status: {type: "committed", queue: saved, proof: {schemaVersion: 1, operationId: request.requestId,
      conversationId: request.conversationId, family: "agent.input_queue.edit", epoch: request.expectedEpoch, sqlFence: "1", replayed: true, result: null}}});
  let queue!:ReturnType<typeof useConversationInputQueue>;
  function Conversation(){queue=useConversationInputQueue(agentContextGateway,empty.conversationId,true,0);return null;}
  const host=document.createElement("div"); document.body.append(host); const root=createRoot(host);
  try {
    await act(async()=>root.render(<StrictMode><AgentQueueProvider><Conversation/></AgentQueueProvider></StrictMode>));
    await act(async()=>{expect(queue.controller.current!.restorePending(request)).toBe(true); expect(await queue.controller.current!.retry()).toBe(true);});
    expect(mocks.conversations.inspectEdit).toHaveBeenCalledExactlyOnceWith(request);
    expect(mocks.conversations.edit).not.toHaveBeenCalled();
    expect(queue.controller.current!.getPendingRequest()).toBeNull();
  } finally {await act(async()=>root.unmount());host.remove();}
});

async function harness(initial: Kind) {
  let draft!: ReturnType<typeof useConversationDraft>;
  let owners!: NonNullable<ReturnType<typeof useAgentQueueOwners>>;
  let queue!: { state: AgentInputQueueState<AgentQueueState> | null; edit: (edit: AgentQueueEdit, acknowledgment?: () => void) => Promise<boolean>; retry: () => Promise<boolean> };
  function Writing({ ready }: { ready: boolean }) {
    draft = useConversationDraft(useAgentDraftStore(), { hostKind: "manuscript", hostId: "22222222-2222-4222-8222-222222222222", conversationId: "11111111-1111-4111-8111-111111111111" });
    const value = useWritingInputQueue("22222222-2222-4222-8222-222222222222", "11111111-1111-4111-8111-111111111111", ready);
    queue = { state: value.state, edit: (edit, ack) => value.controller.current?.edit(edit, ack) ?? Promise.resolve(false), retry: () => value.controller.current?.retry() ?? Promise.resolve(false) };
    return <p>{draft[0]}</p>;
  }
  function Notes({ kind, ready }: { kind: "note" | "readingNote"; ready: boolean }) {
    draft = useConversationDraft(useAgentDraftStore(), { hostKind: kind, hostId: "22222222-2222-4222-8222-222222222222", conversationId: "11111111-1111-4111-8111-111111111111" });
    const value = useNoteInputQueue({ conversationId: "11111111-1111-4111-8111-111111111111", scope: { kind, resourceId: "22222222-2222-4222-8222-222222222222" } }, ready, 0);
    queue = { state: value.state, edit: (edit, ack) => value.edits.current?.edit(edit, ack) ?? Promise.resolve(false), retry: () => value.edits.current?.retry() ?? Promise.resolve(false) };
    return <p>{draft[0]}</p>;
  }
  function Probe({ kind, ready }: { kind: Kind | null; ready: boolean }) {
    owners = useAgentQueueOwners()!;
    return kind === "manuscript" ? <Writing ready={ready} /> : kind ? <Notes key={kind} kind={kind} ready={ready} /> : null;
  }
  const root = createRoot(document.createElement("div"));
  const render = async (kind: Kind | null, ready = true) => { await act(async () => root.render(<StrictMode><AgentQueueProvider><AgentDraftProvider><Probe kind={kind} ready={ready} /></AgentDraftProvider></AgentQueueProvider></StrictMode>)); };
  mocks.writing.read.mockReset().mockResolvedValue(empty); mocks.notes.read.mockReset().mockResolvedValue(empty); mocks.papers.read.mockReset().mockResolvedValue({ ...empty, paperId: "paper-1" });
  await render(initial);
  return { render, get: () => queue, draft: () => draft, owners: () => owners, close: async () => { await act(async () => root.unmount()); } };
}

it.each<Kind>(["manuscript", "note", "readingNote"])("reattaches %s unknown edits before metadata is ready and preserves a retyped draft", async kind => {
  const transport = kind === "manuscript" ? mocks.writing : mocks.notes;
  let fail!: (error: unknown) => void;
  transport.edit.mockReset().mockImplementationOnce(() => new Promise((_resolve, reject) => { fail = reject; })).mockRejectedValueOnce({ code: "closed" }).mockResolvedValue({ ...empty, revision: 1 });
  const h = await harness(kind);
  try {
    await act(async () => h.draft()[1]("original")); let pending!: Promise<boolean>;
    await act(async () => { pending = h.get().edit({ type: "add", inputId: "original-input", text: "original" }, h.draft()[2]("original")); });
    await h.render(null); await act(async () => { fail(new Error("lost reply")); await pending; });
    await h.render(kind, false); expect(h.get().state?.uncertain).toBe(true);
    expect(transport.edit).toHaveBeenCalledTimes(2);
    await h.render(kind); expect(h.get().state?.uncertain).toBe(true);
    await act(async () => h.draft()[1]("changed")); await act(async () => h.draft()[1]("original"));
    await act(async () => { expect(await h.get().edit({ type: "pause" })).toBe(false); await h.get().retry(); });
    expect(transport.edit.mock.calls[0]).toEqual(transport.edit.mock.calls[2]);
    expect(h.draft()[0]).toBe("original"); expect(h.get().state?.uncertain).toBe(false);
  } finally { await h.close(); }
});

it.each<Kind>(["manuscript", "note", "readingNote"])("acknowledges %s exact draft while detached and never resends on reopen", async kind => {
  const transport = kind === "manuscript" ? mocks.writing : mocks.notes;
  let finish!: (value: AgentQueueState) => void;
  transport.edit.mockReset().mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const h = await harness(kind);
  try {
    await act(async () => h.draft()[1]("original")); let pending!: Promise<boolean>;
    await act(async () => { pending = h.get().edit({ type: "add", inputId: "input", text: "original" }, h.draft()[2]("original")); });
    await h.render(null); await act(async () => { finish({ ...empty, revision: 1 }); await pending; });
    await h.render(kind); expect(h.draft()[0]).toBe(""); expect(transport.edit).toHaveBeenCalledOnce();
  } finally { await h.close(); }
});

it("isolates note kinds and writing and forgets all queue owners after confirmed deletion", async () => {
  mocks.writing.edit.mockReset().mockRejectedValue(new Error("unknown")); mocks.notes.edit.mockReset().mockRejectedValue(new Error("unknown"));
  const h = await harness("manuscript");
  try {
    await act(async () => { await h.get().edit({ type: "pause" }); });
    for (const kind of ["note", "readingNote"] as const) {
      await h.render(kind); expect(h.get().state?.uncertain).toBe(false);
      await act(async () => { await h.get().edit({ type: "pause" }); });
    }
    for (const kind of ["manuscript", "note", "readingNote"] as const) { await h.render(kind); expect(h.get().state?.uncertain).toBe(true); }
    h.owners().writing.forgetConversation("11111111-1111-4111-8111-111111111111"); h.owners().notes.forgetConversation("11111111-1111-4111-8111-111111111111"); h.owners().papers.forgetConversation("11111111-1111-4111-8111-111111111111");
    await h.render(null); await h.render("note");
    expect(h.get().state).toBeNull(); expect(await h.get().edit({ type: "pause" })).toBe(false);
    expect(mocks.writing.edit).toHaveBeenCalledTimes(3); expect(mocks.notes.edit).toHaveBeenCalledTimes(6);
  } finally { await h.close(); }
});
