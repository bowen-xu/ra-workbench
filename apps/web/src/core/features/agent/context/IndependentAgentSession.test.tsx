// @vitest-environment happy-dom
import { notify } from "../../../services/notifications";
vi.mock("../../../services/notifications", () => ({ notify: { error: vi.fn(), info: vi.fn(), success: vi.fn(), warning: vi.fn(), dismiss: vi.fn() } }));
import { act, StrictMode, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { WritingDiffDemoHistory } from "../../../../app/previews/agent/WritingDiffDemoHistory";
import { IndependentAgentSession } from "./IndependentAgentSession";
import type { AgentContextGateway } from "../../../services/agentContextGateway";
import { contextStorageKey } from "./contextStorage";
import { recommendationDraftKey, recommendationSubmissionKey } from "./contextRecommendationDraft";
import { AGENT_PREPARATION_FAILURE } from "../agentPreparationFailure";
import { selectAppStorage } from "../../../runtime/appStorage";
import { EMPTY_AGENT_CONTEXT, agentContextKey, selectAgentContext, type AgentContextPreview, type AgentContextPrepareRequest, type AgentContextSelection, type AgentContextQueueEditRequest } from "@research-assistant/core/agent";
const observed = vi.hoisted(() => ({candidates: [] as readonly AgentContextSelection[]}));
vi.mock("./AgentContextProvider", () => ({useAgentContextCandidates:()=>observed.candidates,useAgentContextSuggestions:()=>observed.candidates}));
import { agentCatalogGateway, agentMetadataGateway } from "../../../services/agentGateway";
import { renameProof } from "../conversationRenameTestFacts";
import { resetConversationRenameOwnersForTests } from "../conversationRenameController";
import {MockIndependentReceipts} from "../../../../app/runtime/mock/mockIndependentReceipts";
import {createMockContextConfigurationAuthority} from "../../../../app/runtime/mock/mockContextConfigurationAuthority";
import {IndependentPreparation} from "./independentPreparation";
import {resetAgentConversationProjectionCacheForTests} from "../useAgentConversation";
vi.mock("../../../services/agentGateway", async importOriginal => ({
    ...await importOriginal<typeof import("../../../services/agentGateway")>(),
    agentCatalogGateway: { rename: vi.fn(), renameStatus: vi.fn() }, agentMetadataGateway: { read: vi.fn() },
}));
vi.mock("../presentation/WritingAgentPanel", () => ({ WritingAgentPanel: (props: {
        draft: string;
        onDraftChange: (s: string) => void;
        onSubmit: (s: string, o: object) => void;
        composerAccessory: ReactNode;
        submissionControls: ReactNode;
        inputDisabled: boolean;
        disabledReason?: string;
        onQueue?: (text:string)=>void;
        onCancelSending?: () => void;
        cancelSendingPending?: boolean;
    }) => <div><input value={props.draft} onChange={e => props.onDraftChange(e.target.value)}/><button disabled={props.inputDisabled || !!props.disabledReason} onClick={() => props.onSubmit("Compare", {})}>submit</button>{props.onCancelSending && <button disabled={props.cancelSendingPending} onClick={props.onCancelSending}>stop send</button>}{props.disabledReason}{props.onQueue && <button onClick={()=>props.onQueue?.(props.draft)}>queue</button>}{props.composerAccessory}{props.submissionControls}</div> }));
vi.mock("./AgentContextControl", () => ({ AgentContextControl: () => null }));
const values = new Map<string, string>();
const storage: Storage = { get length() { return values.size; }, key: i => [...values.keys()][i] ?? null, getItem: key => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value); }, removeItem: key => { values.delete(key); }, clear: () => values.clear() };
vi.stubGlobal("localStorage", storage);
(globalThis as typeof globalThis & {
    IS_REACT_ACT_ENVIRONMENT?: boolean;
}).IS_REACT_ACT_ENVIRONMENT = true;
const id = "11111111-1111-4111-8111-111111111111", other = "22222222-2222-4222-8222-222222222222";
const source = { resource: { kind: "note" as const, resourceId: other }, label: "Synthetic note" };
let cleanup: (() => Promise<void>) | undefined;
let originalLocks: PropertyDescriptor | undefined;
afterEach(async () => { await cleanup?.(); if (originalLocks) Object.defineProperty(navigator, "locks", originalLocks); else delete (navigator as {locks?:unknown}).locks; originalLocks = undefined; resetAgentConversationProjectionCacheForTests(); resetConversationRenameOwnersForTests(localStorage); localStorage.clear(); observed.candidates=[];vi.mocked(agentCatalogGateway.rename).mockReset(); vi.mocked(agentMetadataGateway.read).mockReset(); });
function gateway() {
    const history = new WritingDiffDemoHistory();
    const frozen: AgentContextPreview = { schemaVersion: 1, conversationId: id, requestId: crypto.randomUUID(), turnId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID(), input: { text: "Compare", model: { providerId: "deepseek", modelId: "deepseek-v4-pro", revision: null }, reasoningEffort: null, context: [] } };
    const prepare = vi.fn(async (request: AgentContextPrepareRequest) => ({ ...frozen, requestId: request.requestId }));
    const start = vi.fn(async (preview: AgentContextPreview) => ({ schemaVersion: 1 as const, conversationId: id, requestId: preview.requestId, turnId: preview.turnId, alreadyStarted: false, outcome: "completed" as const }));
    const port: AgentContextGateway = { sessions: history.gateway, list: async () => [], search: async () => [], prepare, restore: async () => frozen, start, interrupt: async () => undefined };
    return { port, prepare, start, frozen, history };
}
async function mount(port: AgentContextGateway, messageRequest?: import("../AgentCanvasMessage").AgentCanvasMessage, archived = false,
    options: Partial<Pick<Parameters<typeof IndependentAgentSession>[0], "title" | "onRenamed" | "onRefreshTitle" | "pending" | "ensureCreated">> = {}) {
    selectAppStorage(localStorage);
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    cleanup = async () => { await act(async () => root.unmount()); host.remove(); };
    await act(async () => { root.render(<StrictMode><IndependentAgentSession archived={archived} messageRequest={messageRequest ?? null} id={id} title="Synthetic" gateway={port} open onOpenChange={() => undefined} headerControls={null} onStarted={() => undefined} {...options}/></StrictMode>); });
    return host;
}
async function retryNotice() {
    const action = [...vi.mocked(notify.error).mock.calls].reverse().find(call => call[1]?.action)?.[1]?.action;
    expect(action && typeof action === "object" && "onClick" in action).toBe(true);
    if (action && typeof action === "object" && "onClick" in action && typeof action.onClick === "function")
        await act(async () => { (action.onClick as () => void)(); });
}
async function click(host: HTMLElement, text: string) { const button = [...host.querySelectorAll("button")].find(b => b.textContent === text); expect(button).toBeDefined(); await act(async () => button!.click()); }
async function unmount() {await cleanup?.(); cleanup = undefined;}

it("shows the submitted user bubble while preparation is pending and restores editing after a definite failure", async () => {
    const f = gateway(); let reject!: (error: unknown) => void;
    f.prepare.mockImplementationOnce(() => new Promise((_, fail) => {reject = fail;}));
    localStorage.setItem(`research-assistant:independent-draft:${id}`, "Compare");
    const host = await mount(f.port);
    await click(host, "submit");
    await vi.waitFor(() => expect(reject).toBeTypeOf("function"));
    expect(host.querySelector('[data-pending-submission="true"]')?.textContent).toBe("Compare");
    expect([...host.querySelectorAll("button")].find(button => button.textContent === "submit")?.disabled).toBe(true);
    await act(async () => {reject("modelUnavailable");});
    await vi.waitFor(() => expect(host.querySelector('[data-pending-submission="true"]')).toBeNull());
    expect((host.querySelector("input") as HTMLInputElement).value).toBe("Compare");
    expect([...host.querySelectorAll("button")].find(button => button.textContent === "submit")?.disabled).toBe(false);
});

it("keeps the submitted bubble and input locked while start is pending, then preserves it until history catches up", async () => {
    const f = gateway(); let release!: (receipt: Awaited<ReturnType<typeof f.start>>) => void;
    f.start.mockImplementationOnce(() => new Promise(resolve => {release = resolve;}));
    localStorage.setItem(`research-assistant:independent-draft:${id}`, "Compare");
    const host = await mount(f.port);
    await click(host, "submit");
    await vi.waitFor(() => expect(release).toBeTypeOf("function"));
    expect(host.querySelector('[data-pending-submission="true"]')?.textContent).toBe("Compare");
    expect([...host.querySelectorAll("button")].find(button => button.textContent === "submit")?.disabled).toBe(true);
    const frozen = await f.prepare.mock.results[0]!.value;
    await act(async () => {release({schemaVersion:1,conversationId:id,requestId:frozen.requestId,turnId:frozen.turnId,alreadyStarted:false,outcome:"completed"});});
    await vi.waitFor(() => expect([...host.querySelectorAll("button")].find(button => button.textContent === "submit")?.disabled).toBe(false));
    expect(host.querySelector('[data-pending-submission="true"]')?.textContent).toBe("Compare");
});

it("replaces the pending bubble when its own turn appears in conversation history", async () => {
    const f = gateway();
    f.start.mockImplementationOnce(async frozen => {
        f.history.start({scope:{manuscriptId:"independent",conversationId:id,requestId:frozen.requestId,contextSnapshotId:frozen.contextSnapshotId},turnId:frozen.turnId}, frozen.input.text, frozen.input.model);
        return {schemaVersion:1,conversationId:id,requestId:frozen.requestId,turnId:frozen.turnId,alreadyStarted:false,outcome:"completed"};
    });
    localStorage.setItem(`research-assistant:independent-draft:${id}`, "Compare");
    const host = await mount(f.port);
    await click(host, "submit");
    await vi.waitFor(() => expect(f.start).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(host.querySelector('[data-pending-submission="true"]')).toBeNull());
});

it("retains an accepted pending bubble and original recovery identity if local finish throws", async () => {
    const f = gateway(), receipts = new MockIndependentReceipts(localStorage, f.prepare, frozen => f.start(frozen), () => []);
    f.start.mockImplementation(async frozen => ({schemaVersion:1,conversationId:id,requestId:frozen.requestId,turnId:frozen.turnId,alreadyStarted:true,outcome:"completed"}));
    const finish = vi.spyOn(IndependentPreparation.prototype, "finish").mockRejectedValueOnce(new Error("local finish interrupted"));
    const port = {...f.port, preparation:{proven:receipts.proven,status:receipts.status},guardedStart:receipts.guardedStart};
    localStorage.setItem(`research-assistant:independent-draft:${id}`, "Compare");
    const host = await mount(port);
    try {
        await click(host, "submit");
        await vi.waitFor(() => expect(f.start).toHaveBeenCalledOnce());
        await vi.waitFor(() => expect(finish).toHaveBeenCalledOnce());
        expect(host.querySelector('[data-pending-submission="true"]')?.textContent).toBe("Compare");
        expect(localStorage.getItem(`research-assistant:independent-preparation:v1:${id}`)).not.toBeNull();
        expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("发送结果尚未确认"), expect.objectContaining({action:expect.anything()}));
        expect(host.querySelector("input")?.value).toBe("Compare");
    } finally {finish.mockRestore(); f.history.dispose();}
});

it("does not carry an unaccepted bubble or late preparation into a reopened session", async () => {
    const f = gateway(); let release!: (preview: AgentContextPreview) => void;
    f.prepare.mockImplementationOnce(() => new Promise(resolve => {release = resolve;}));
    const first = await mount(f.port);
    await click(first, "submit");
    await vi.waitFor(() => expect(release).toBeTypeOf("function"));
    expect(first.querySelector('[data-pending-submission="true"]')).not.toBeNull();
    await unmount();
    const reopened = await mount(f.port);
    await act(async () => {release(f.frozen);});
    expect(reopened.querySelector('[data-pending-submission="true"]')).toBeNull();
    expect(f.start).not.toHaveBeenCalled();
});

it("holds new admission until the original task has finished its owner cleanup", async () => {
    const f = gateway(), runId = crypto.randomUUID();
    const start = vi.fn(async (preview: AgentContextPreview) => ({schemaVersion:1 as const, conversationId:id,
        requestId:preview.requestId, turnId:preview.turnId, runId, alreadyStarted:false, outcome:"completed" as const}));
    let complete!: () => void;
    const observe = vi.fn((identity: {conversationId:string;turnId:string;runId:string}) => new Promise<{
        schemaVersion:1; conversationId:string; turnId:string; runId:string;
        status:{type:"finished";completion:"completed";diagnostic:null}}>(resolve => {
        complete = () => resolve({schemaVersion:1,...identity,status:{type:"finished",completion:"completed",diagnostic:null}});
    }));
    localStorage.setItem(`research-assistant:independent-draft:${id}`, "Compare");
    const host = await mount({...f.port, start, taskObservation:{observe}});
    await click(host, "submit");
    await vi.waitFor(() => expect(host.textContent).toContain("原运行正在收尾"));
    await click(host, "submit"); expect(start).toHaveBeenCalledOnce();
    await act(async () => complete());
    await vi.waitFor(() => expect(host.textContent).not.toContain("原运行正在收尾"));
    await unmount(); f.history.dispose();
});

it.each([false, true])("preserves and displays the original task recovery without resending (replayed=%s)", async alreadyStarted => {
    const f = gateway(), runId = crypto.randomUUID();
    const start = vi.fn(async (preview: AgentContextPreview) => ({schemaVersion: 1 as const, conversationId: id,
        requestId: preview.requestId, turnId: preview.turnId, runId, alreadyStarted, outcome: null}));
    const observe = vi.fn(async (identity: {conversationId:string;turnId:string;runId:string}) => ({schemaVersion:1 as const, ...identity,
        status: {type:"finished" as const, completion:"recoveryRequired" as const,
            diagnostic:{code:"context_persistence_failure",phase:"write",safeCause:"sql_write_failed",operationId:runId,sqliteCode:"ConstraintViolation",sqliteExtendedCode:1811}}}));
    const port: AgentContextGateway = {...f.port, start, taskObservation: {observe}};
    localStorage.setItem(`research-assistant:independent-draft:${id}`, "Compare");
    const host = await mount(port); await click(host, "submit");
    await vi.waitFor(() => expect(host.textContent).toContain("原运行需要核对"));
    const writeText = vi.fn(async (_value: string) => undefined);
    Object.defineProperty(navigator, "clipboard", {configurable: true, value: {writeText}});
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="复制诊断信息"]')!.click());
    const diagnostic = writeText.mock.lastCall?.[0] ?? "";
    expect(diagnostic).toContain("sql_write_failed");
    expect(diagnostic).toContain("sqlite=1811");
    expect(diagnostic).toContain("sqliteCode=ConstraintViolation");
    expect(diagnostic).toContain(`operation=${runId}`);
    expect(start).toHaveBeenCalledOnce();
    expect(JSON.parse(localStorage.getItem(`research-assistant:agent-task-observation:v1:${id}`)!)).toMatchObject({conversationId:id,runId});
    await click(host, "submit"); expect(start).toHaveBeenCalledOnce();
    await unmount(); const reopened = await mount(port);
    await vi.waitFor(() => expect(reopened.textContent).toContain("原运行需要核对"));
    expect(start).toHaveBeenCalledOnce();
    await unmount(); f.history.dispose();
});

it("recovers a guarded lost start without restarting or consuming a later identical draft", async () => {
    const f = gateway(), key = `research-assistant:independent-draft:${id}`;
    localStorage.setItem(key, "Compare");
    f.start.mockImplementation(async frozen => {
        f.history.start({scope: {manuscriptId: "independent", conversationId: id, requestId: frozen.requestId, contextSnapshotId: frozen.contextSnapshotId}, turnId: frozen.turnId}, frozen.input.text, frozen.input.model);
        return {schemaVersion: 1, conversationId: id, requestId: frozen.requestId, turnId: frozen.turnId, alreadyStarted: false, outcome: "completed"};
    });
    const receipts = new MockIndependentReceipts(localStorage, f.prepare, frozen => f.start(frozen), conversation => f.history.snapshot().events.get(conversation) ?? []);
    const start = vi.fn(async (request: Parameters<typeof receipts.guardedStart.start>[0], epoch: string) => {
        await receipts.guardedStart.start(request, epoch); throw new Error("lost start response");
    });
    const port = {...f.port, preparation: {proven: receipts.proven, status: receipts.status}, guardedStart: {...receipts.guardedStart, start}};
    const host = await mount(port); await click(host, "submit");
    expect(start).toHaveBeenCalledOnce();
    const input = host.querySelector<HTMLInputElement>("input")!;
    for (const text of ["Other draft", "Compare"]) await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, text);
        input.dispatchEvent(new Event("input", {bubbles: true}));
    });
    await unmount();
    await mount(port); await retryNotice();
    expect(start).toHaveBeenCalledOnce(); expect(localStorage.getItem(key)).toBe("Compare");
    await vi.waitFor(() => expect(localStorage.getItem(`research-assistant:independent-preparation:v1:${id}`)).toBeNull());
    await unmount(); f.history.dispose();
});
it("preserves safe queue admission transaction diagnostics when start is unknown", async () => {
    const f = gateway();
    const receipts = new MockIndependentReceipts(localStorage, f.prepare, frozen => f.start(frozen), conversation => f.history.snapshot().events.get(conversation) ?? []);
    const start = vi.fn(async (request: Parameters<typeof receipts.guardedStart.start>[0]) => {
        throw {code:"storage",reason:{code:"DATA_SQL_COMMIT_UNKNOWN",reason:{operationId:request.requestId,epoch:null,sqlFence:null,
            phase:"commit",sqliteCode:"ConstraintViolation",sqliteExtendedCode:787,safeCause:"sql_commit_outcome_unknown"}}};
    });
    const port = {...f.port, preparation:{proven:receipts.proven,status:receipts.status},guardedStart:{...receipts.guardedStart,start}};
    vi.mocked(notify.error).mockClear();
    const host = await mount(port); await click(host,"submit");
    await vi.waitFor(()=>expect(vi.mocked(notify.error).mock.calls.some(([text]) => typeof text === "string" && text.includes("DATA_SQL_COMMIT_UNKNOWN")
        && text.includes("phase=commit") && text.includes("cause=sql_commit_outcome_unknown") && text.includes("sqlite=ConstraintViolation")
        && text.includes("sqliteExtended=787"))).toBe(true));
    expect(start).toHaveBeenCalledOnce();await unmount();f.history.dispose();
});
it.each([false, true])("keeps the accepted start until a context save with a lost reply is confirmed (committed=%s)", async committed => {
    const locks = new Set<string>();
    originalLocks = Object.getOwnPropertyDescriptor(navigator, "locks");
    Object.defineProperty(navigator, "locks", {configurable:true, value:{request:async (name:string, _options:unknown, callback:(lock:object|null)=>Promise<unknown>) => {
        if (locks.has(name)) return callback(null); locks.add(name); try {return await callback({});} finally {locks.delete(name);}
    }}});
    const f = gateway(), key = `research-assistant:independent-preparation:v1:${id}`;
    const rows = new Map([[id, {revision:1, state:EMPTY_AGENT_CONTEXT}]]);
    const authority = await createMockContextConfigurationAuthority(localStorage, rows);
    const originalSave = authority.configurationSave.save;
    const save = vi.fn(async (...args:Parameters<typeof originalSave>) => {
        if (save.mock.calls.length === 1) {
            if (committed) await originalSave(...args);
            throw new Error("lost context receipt");
        }
        return originalSave(...args);
    });
    localStorage.setItem(recommendationDraftKey(id), JSON.stringify([{item:source, selected:true}]));
    localStorage.setItem(`research-assistant:independent-draft:${id}`, "Compare");
    f.start.mockImplementation(async frozen => {
        f.history.start({scope:{manuscriptId:"independent",conversationId:id,requestId:frozen.requestId,contextSnapshotId:frozen.contextSnapshotId},turnId:frozen.turnId}, frozen.input.text, frozen.input.model);
        return {schemaVersion:1,conversationId:id,requestId:frozen.requestId,turnId:frozen.turnId,alreadyStarted:true,outcome:"completed"};
    });
    const receipts = new MockIndependentReceipts(localStorage, f.prepare, frozen => f.start(frozen), conversation => f.history.snapshot().events.get(conversation) ?? []);
    const port = {...f.port, ...authority, configurationSave:{...authority.configurationSave, save},
        preparation:{proven:receipts.proven,status:receipts.status}, guardedStart:receipts.guardedStart};
    const host = await mount(port);
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });
    await click(host, "submit");
    expect(f.prepare).toHaveBeenCalledOnce();
    expect(f.start).toHaveBeenCalledOnce();
    await expect(f.start.mock.results[0]!.value).resolves.toMatchObject({outcome:"completed"});
    await vi.waitFor(() => expect(save).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(host.textContent).toContain("上下文保存结果尚未确认"));
    expect(localStorage.getItem(key)).not.toBeNull();
    expect(localStorage.getItem(recommendationSubmissionKey(id))).not.toBeNull();
    expect(localStorage.getItem(recommendationDraftKey(id))).not.toBe("[]");
    expect(f.start).toHaveBeenCalledOnce();
    localStorage.setItem(`research-assistant:independent-draft:${id}`, "Later draft");
    localStorage.setItem(`research-assistant:independent-draft-version:${id}`, crypto.randomUUID());
    await unmount();
    const reopened = await mount(port);
    await vi.waitFor(() => expect(reopened.textContent).toContain("上下文保存结果尚未确认"));
    await click(reopened, "核对并重试");
    await click(reopened, "核对原请求");
    await vi.waitFor(() => expect(localStorage.getItem(key)).toBeNull());
    expect(rows.get(id)?.state.persistent).toEqual([source]);
    expect(rows.get(id)?.revision).toBe(2);
    expect(localStorage.getItem(recommendationSubmissionKey(id))).toBeNull();
    expect(localStorage.getItem(`research-assistant:independent-draft:${id}`)).toBe("Later draft");
    expect(save).toHaveBeenCalledTimes(2);
    expect(save.mock.calls[1]).toEqual(save.mock.calls[0]);
    expect(f.start).toHaveBeenCalledOnce();
    await unmount(); f.history.dispose();
});
it("observes the original owner rediscovered after a lost start response", async () => {
    const f = gateway(), runId = crypto.randomUUID();
    localStorage.setItem(`research-assistant:independent-draft:${id}`, "Compare");
    f.start.mockImplementation(async frozen => {
        f.history.start({scope:{manuscriptId:"independent",conversationId:id,requestId:frozen.requestId,contextSnapshotId:frozen.contextSnapshotId},turnId:frozen.turnId}, frozen.input.text, frozen.input.model);
        return {schemaVersion:1,conversationId:id,requestId:frozen.requestId,turnId:frozen.turnId,alreadyStarted:false,outcome:"completed"};
    });
    const receipts = new MockIndependentReceipts(localStorage, f.prepare, frozen => f.start(frozen), conversation => f.history.snapshot().events.get(conversation) ?? []);
    const start = vi.fn(async (request: Parameters<typeof receipts.guardedStart.start>[0], epoch:string) => {
        await receipts.guardedStart.start(request, epoch); throw new Error("lost start response");
    });
    const read = vi.fn(async (...args:Parameters<typeof receipts.guardedStart.read>) => {
        const response = await receipts.guardedStart.read(...args);
        return response.status.type === "started" ? {...response, status:{...response.status,result:{...response.status.result,runId}}} : response;
    });
    const observe = vi.fn(() => new Promise<Awaited<ReturnType<NonNullable<AgentContextGateway["taskObservation"]>["observe"]>>>(() => undefined));
    const port:AgentContextGateway = {...f.port, preparation:{proven:receipts.proven,status:receipts.status}, guardedStart:{start,read}, taskObservation:{observe}};
    const host = await mount(port); await click(host, "submit");
    await unmount(); await mount(port); await retryNotice();
    await vi.waitFor(() => expect(observe).toHaveBeenCalledWith(expect.objectContaining({conversationId:id,runId})));
    expect(JSON.parse(localStorage.getItem(`research-assistant:agent-task-observation:v1:${id}`)!)).toMatchObject({conversationId:id,runId});
    expect(start).toHaveBeenCalledOnce();
    await unmount(); f.history.dispose();
});
it("keeps the original request and draft when the started task is not discoverable", async () => {
    const f = gateway(), draftKey = `research-assistant:independent-draft:${id}`;
    localStorage.setItem(draftKey, "Compare");
    f.start.mockImplementation(async frozen => {
        f.history.start({scope:{manuscriptId:"independent",conversationId:id,requestId:frozen.requestId,contextSnapshotId:frozen.contextSnapshotId},turnId:frozen.turnId}, frozen.input.text, frozen.input.model);
        return {schemaVersion:1,conversationId:id,requestId:frozen.requestId,turnId:frozen.turnId,alreadyStarted:false,outcome:"completed"};
    });
    const receipts = new MockIndependentReceipts(localStorage, f.prepare, frozen => f.start(frozen), conversation => f.history.snapshot().events.get(conversation) ?? []);
    const start = vi.fn(async (request: Parameters<typeof receipts.guardedStart.start>[0], epoch:string) => {
        await receipts.guardedStart.start(request, epoch); throw new Error("lost start response");
    });
    const read = vi.fn(async (...args:Parameters<typeof receipts.guardedStart.read>) => {
        const response = await receipts.guardedStart.read(...args);
        return response.status.type === "started" ? {...response, status:{...response.status,type:"startedUnobserved" as const}} : response;
    });
    const port:AgentContextGateway = {...f.port, preparation:{proven:receipts.proven,status:receipts.status}, guardedStart:{start,read}};
    const host = await mount(port); await click(host, "submit"); await unmount();
    const reopened = await mount(port); await retryNotice();
    await vi.waitFor(() => expect(vi.mocked(notify.error).mock.calls.some(call => String(call[0]).includes("AGENT_TASK_IDENTITY_UNAVAILABLE"))).toBe(true));
    expect(reopened.textContent).toContain("原请求已启动，但暂时无法观察运行状态");
    expect(localStorage.getItem(draftKey)).toBe("Compare");
    expect(localStorage.getItem(`research-assistant:independent-preparation:v1:${id}`)).not.toBeNull();
    await click(reopened, "submit"); expect(start).toHaveBeenCalledOnce();
    await unmount(); const restored = await mount(port);
    expect(restored.textContent).toContain("原请求已启动，但暂时无法观察运行状态");
    await click(restored, "核对原请求"); expect(start).toHaveBeenCalledOnce();
    await unmount(); f.history.dispose();
});
it("clears the matching observation locator after durable owner settlement without restarting", async () => {
    const f = gateway(), runId = crypto.randomUUID(), draftKey = `research-assistant:independent-draft:${id}`;
    localStorage.setItem(draftKey, "Compare");
    f.start.mockImplementation(async frozen => {
        f.history.start({scope:{manuscriptId:"independent",conversationId:id,requestId:frozen.requestId,contextSnapshotId:frozen.contextSnapshotId},turnId:frozen.turnId}, frozen.input.text, frozen.input.model);
        return {schemaVersion:1,conversationId:id,requestId:frozen.requestId,turnId:frozen.turnId,alreadyStarted:false,outcome:"completed"};
    });
    const receipts = new MockIndependentReceipts(localStorage, f.prepare, frozen => f.start(frozen), conversation => f.history.snapshot().events.get(conversation) ?? []);
    const start = vi.fn(async (request: Parameters<typeof receipts.guardedStart.start>[0], epoch:string) => {
        await receipts.guardedStart.start(request, epoch); throw new Error("lost start response");
    });
    let settled = false;
    const read = vi.fn(async (...args:Parameters<typeof receipts.guardedStart.read>) => {
        const response = await receipts.guardedStart.read(...args);
        if (response.status.type !== "started") return response;
        return {...response, status:{...response.status,type:settled ? "startedSettled" as const : "startedUnobserved" as const,
            result:{...response.status.result,outcome:settled ? "completed" as const : null}}};
    });
    const observe = vi.fn(async (identity:{conversationId:string;turnId:string;runId:string}) => ({schemaVersion:1 as const,...identity,status:{type:"unavailable" as const}}));
    const port:AgentContextGateway = {...f.port, preparation:{proven:receipts.proven,status:receipts.status}, guardedStart:{start,read}, taskObservation:{observe}};
    const host = await mount(port); await click(host, "submit"); await unmount();
    await mount(port); await retryNotice(); await unmount();
    const saved = JSON.parse(localStorage.getItem(`research-assistant:independent-preparation:v1:${id}`)!);
    const locatorKey = `research-assistant:agent-task-observation:v1:${id}`;
    localStorage.setItem(locatorKey, JSON.stringify({conversationId:id,turnId:saved.start.result.turnId,runId}));
    settled = true; const restored = await mount(port); await click(restored, "核对原请求");
    await vi.waitFor(() => expect(localStorage.getItem(locatorKey)).toBeNull());
    await vi.waitFor(() => expect(localStorage.getItem(`research-assistant:independent-preparation:v1:${id}`)).toBeNull());
    expect(start).toHaveBeenCalledOnce();
    await unmount(); f.history.dispose();
});
it("checks a lost preparation reply and waits for an explicit continue before starting", async () => {
    const f = gateway(); localStorage.setItem(`research-assistant:independent-draft:${id}`, "Compare");
    f.start.mockImplementation(async frozen => ({schemaVersion: 1, conversationId: id, requestId: frozen.requestId, turnId: frozen.turnId, alreadyStarted: true, outcome: "completed"}));
    const receipts = new MockIndependentReceipts(localStorage, f.prepare, frozen => f.start(frozen), () => []);
    const prepare = vi.fn(async (...args: Parameters<typeof receipts.proven.prepare>) => {await receipts.proven.prepare(...args); throw new Error("lost preparation response");});
    const start = vi.fn(receipts.guardedStart.start);
    const port = {...f.port, preparation: {proven: {prepare}, status: receipts.status}, guardedStart: {...receipts.guardedStart, start}};
    const host = await mount(port); await click(host, "submit");
    expect(prepare).toHaveBeenCalledOnce(); expect(start).not.toHaveBeenCalled();
    await unmount(); await mount(port); await retryNotice();
    expect(start).not.toHaveBeenCalled(); expect(prepare).toHaveBeenCalledOnce();
    await retryNotice(); expect(start).toHaveBeenCalledOnce(); expect(prepare).toHaveBeenCalledOnce();
    await act(async () => {await start.mock.results[0]!.value;});
    await vi.waitFor(() => expect(localStorage.getItem(`research-assistant:independent-preparation:v1:${id}`)).toBeNull());
    await unmount(); f.history.dispose();
});
it("explicitly resumes an admitted paused queue through the production session notification", async () => {
    vi.mocked(notify.error).mockClear();
    const f = gateway(), key = `research-assistant:independent-preparation:v1:${id}`, inputId = crypto.randomUUID();
    f.start.mockImplementation(async frozen => ({schemaVersion: 1, conversationId: id, requestId: frozen.requestId, turnId: frozen.turnId, alreadyStarted: true, outcome: "completed"}));
    const preview = {...f.frozen, queue: {inputId, expectedRevision: 2}};
    const receipts = new MockIndependentReceipts(localStorage, async request => ({...preview, requestId: request.requestId}),
        frozen => f.start(frozen), () => [], undefined, () => true);
    const start = vi.fn(receipts.guardedStart.start).mockRejectedValueOnce(new Error("lost start"));
    const controller = new IndependentPreparation(localStorage, key, id, receipts.status, receipts.proven, undefined, {start, read: receipts.guardedStart.read});
    await controller.begin({conversationId: id, requestId: preview.requestId, text: "Compare", resources: [], queue: {inputId, expectedRevision: 1}}, true);
    await controller.start(); expect(controller.getSnapshot().phase).toBe("unknown");
    let queue = {schemaVersion: 1 as const, conversationId: id, revision: 4, paused: true, preparingInputId: null, inputs: []};
    const read = vi.fn(async () => queue), edit = vi.fn(async (request: AgentContextQueueEditRequest) => {
        expect(request.expectedEpoch).toBe(receipts.epoch); expect(request.edit).toEqual({type: "resume"});
        expect(JSON.parse(localStorage.getItem(key)!).resume.request).toEqual(request);
        queue = {...queue, revision: queue.revision + 1, paused: false}; return queue;
    });
    const port = {...f.port, queue: {read, edit}, preparation: {proven: receipts.proven, status: receipts.status}, guardedStart: {start, read: receipts.guardedStart.read}};
    await mount(port); await retryNotice();
    expect(start).toHaveBeenCalledOnce(); expect(edit).not.toHaveBeenCalled();
    await vi.waitFor(() => expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("队列消息已接纳"), expect.anything()));
    const oldAction = [...vi.mocked(notify.error).mock.calls].reverse().find(call => call[1]?.action)?.[1]?.action;
    await retryNotice();
    expect(edit).toHaveBeenCalledOnce(); expect(start).toHaveBeenCalledTimes(2);
    await expect(start.mock.results[1]!.value).resolves.toMatchObject({turnId: preview.turnId});
    await vi.waitFor(() => expect(localStorage.getItem(key)).toBeNull());
    expect(notify.dismiss).toHaveBeenCalledWith(key);
    const notices = vi.mocked(notify.error).mock.calls.length;
    if (oldAction && typeof oldAction === "object" && "onClick" in oldAction && typeof oldAction.onClick === "function") await act(async () => (oldAction.onClick as () => void)());
    expect(start).toHaveBeenCalledTimes(2); expect(vi.mocked(notify.error).mock.calls.length).toBe(notices);
    await unmount(); f.history.dispose();
});
it("finishes a settled legacy queued request through the production recovery action", async () => {
    const f = gateway(), key = `research-assistant:independent-preparation:v1:${id}`, inputId = crypto.randomUUID();
    const preview = {...f.frozen, queue: {inputId, expectedRevision: 2}};
    const receipts = new MockIndependentReceipts(localStorage, async request => ({...preview, requestId: request.requestId}),
        frozen => f.start(frozen), () => [], undefined, () => true);
    const start = vi.fn(receipts.guardedStart.start).mockRejectedValueOnce(new Error("lost start"));
    const controller = new IndependentPreparation(localStorage, key, id, receipts.status, receipts.proven, undefined, {start, read: receipts.guardedStart.read});
    await controller.begin({conversationId: id, requestId: preview.requestId, text: "Compare", resources: [], queue: {inputId, expectedRevision: 1}}, true);
    await controller.start(); expect(controller.getSnapshot().phase).toBe("unknown");
    const read = vi.fn(async (request: Parameters<typeof receipts.guardedStart.read>[0], expectedEpoch: string) => ({
        schemaVersion: 1 as const, intent: request, expectedEpoch, status: {type: "legacyStartedSettled" as const,
            result: {schemaVersion: 1 as const, conversationId: id, requestId: request.requestId, turnId: preview.turnId,
                alreadyStarted: true, outcome: "completed" as const}},
    }));
    const port = {...f.port, preparation: {proven: receipts.proven, status: receipts.status}, guardedStart: {start, read}};
    await mount(port); await retryNotice();
    await vi.waitFor(() => expect(localStorage.getItem(key)).toBeNull());
    expect(read).toHaveBeenCalledOnce(); expect(start).toHaveBeenCalledOnce();
    await unmount(); f.history.dispose();
});
it("observes the original live owner for a legacy queued request", async () => {
    const f = gateway(), key = `research-assistant:independent-preparation:v1:${id}`,
        inputId = crypto.randomUUID(), runId = crypto.randomUUID();
    const preview = {...f.frozen, queue: {inputId, expectedRevision: 2}};
    const receipts = new MockIndependentReceipts(localStorage, async request => ({...preview, requestId: request.requestId}),
        frozen => f.start(frozen), () => [], undefined, () => true);
    const start = vi.fn(receipts.guardedStart.start).mockRejectedValueOnce(new Error("lost start"));
    const controller = new IndependentPreparation(localStorage, key, id, receipts.status, receipts.proven, undefined, {start, read: receipts.guardedStart.read});
    await controller.begin({conversationId: id, requestId: preview.requestId, text: "Compare", resources: [], queue: {inputId, expectedRevision: 1}}, true);
    await controller.start();
    const read = vi.fn(async (request: Parameters<typeof receipts.guardedStart.read>[0], expectedEpoch: string) => ({
        schemaVersion: 1 as const, intent: request, expectedEpoch, status: {type: "legacyStartedObserved" as const,
            result: {schemaVersion: 1 as const, conversationId: id, requestId: request.requestId, turnId: preview.turnId,
                runId, alreadyStarted: true, outcome: null}},
    }));
    const observe = vi.fn(() => new Promise<Awaited<ReturnType<NonNullable<AgentContextGateway["taskObservation"]>["observe"]>>>(() => undefined));
    const port: AgentContextGateway = {...f.port, preparation: {proven: receipts.proven, status: receipts.status}, guardedStart: {start, read}, taskObservation: {observe}};
    await mount(port); await retryNotice();
    await vi.waitFor(() => expect(observe).toHaveBeenCalledWith({conversationId: id, turnId: preview.turnId, runId}));
    expect(JSON.parse(localStorage.getItem(`research-assistant:agent-task-observation:v1:${id}`)!)).toMatchObject({turnId: preview.turnId, runId});
    expect(localStorage.getItem(key)).toBeNull();
    expect(start).toHaveBeenCalledOnce();
    await unmount(); f.history.dispose();
});
it("allows editing after the guarded native pre-transaction model rejection", async () => {
    const f = gateway(); localStorage.setItem(`research-assistant:independent-draft:${id}`, "Compare");
    const receipts = new MockIndependentReceipts(localStorage, f.prepare, frozen => f.start(frozen), () => []);
    const prepare = vi.fn().mockRejectedValue({code: "modelUnavailable"});
    const port = {...f.port, preparation: {proven: {prepare}, status: receipts.status}, guardedStart: receipts.guardedStart};
    const host = await mount(port); await click(host, "submit");
    expect(host.querySelector<HTMLInputElement>("input")?.value).toBe("Compare");
    expect(localStorage.getItem(`research-assistant:independent-preparation:v1:${id}`)).toBeNull();
    expect(vi.mocked(notify.error)).toHaveBeenCalledWith(expect.stringContaining("当前模型未配置"));
    expect(f.start).not.toHaveBeenCalled();
    await unmount(); f.history.dispose();
});
it("does not begin a guarded request after its preparing view is unmounted", async () => {
    const f = gateway(); localStorage.setItem(`research-assistant:independent-draft:${id}`, "Compare");
    const receipts = new MockIndependentReceipts(localStorage, f.prepare, frozen => f.start(frozen), () => []);
    const prepare = vi.fn(receipts.proven.prepare);
    const port = {...f.port, preparation: {proven: {prepare}, status: receipts.status}, guardedStart: receipts.guardedStart};
    let finishCreation!: () => void;
    const ensureCreated = vi.fn(() => new Promise<void>(resolve => {finishCreation = resolve;}));
    const host = await mount(port, undefined, false, {pending: true, ensureCreated});
    await click(host, "submit"); expect(ensureCreated).toHaveBeenCalledOnce();
    await unmount(); await act(async () => finishCreation());
    expect(prepare).not.toHaveBeenCalled(); expect(f.start).not.toHaveBeenCalled();
    expect(localStorage.getItem(`research-assistant:independent-preparation:v1:${id}`)).toBeNull();
    f.history.dispose();
});

it("stops local send preflight without starting the durable request or consuming the draft", async () => {
    const f = gateway(); localStorage.setItem(`research-assistant:independent-draft:${id}`, "Compare");
    const receipts = new MockIndependentReceipts(localStorage, f.prepare, frozen => f.start(frozen), () => []);
    const prepare = vi.fn(receipts.proven.prepare);
    const port = {...f.port, preparation: {proven: {prepare}, status: receipts.status}, guardedStart: receipts.guardedStart};
    let finishCreation!: () => void;
    const ensureCreated = vi.fn(() => new Promise<void>(resolve => {finishCreation = resolve;}));
    const host = await mount(port, undefined, false, {pending: true, ensureCreated});
    await click(host, "submit");
    expect(host.querySelector('[data-pending-submission="true"]')?.textContent).toBe("Compare");
    await click(host, "stop send");
    expect([...host.querySelectorAll("button")].find(button => button.textContent === "stop send")?.disabled).toBe(true);
    expect(prepare).not.toHaveBeenCalled();
    await act(async () => finishCreation());
    await vi.waitFor(() => expect(host.querySelector('[data-pending-submission="true"]')).toBeNull());
    expect(prepare).not.toHaveBeenCalled(); expect(f.start).not.toHaveBeenCalled();
    expect(localStorage.getItem(`research-assistant:independent-preparation:v1:${id}`)).toBeNull();
    expect(localStorage.getItem(`research-assistant:independent-draft:${id}`)).toBe("Compare");
    expect(host.querySelector<HTMLInputElement>("input")?.value).toBe("Compare");
    await unmount(); f.history.dispose();
});

it("does not offer preflight cancellation after native preparation has begun", async () => {
    const f = gateway(); localStorage.setItem(`research-assistant:independent-draft:${id}`, "Compare");
    let finishPrepare!: (preview: AgentContextPreview) => void;
    f.prepare.mockImplementationOnce(() => new Promise(resolve => {finishPrepare = resolve;}));
    const host = await mount(f.port);
    await click(host, "submit");
    await vi.waitFor(() => expect(f.prepare).toHaveBeenCalledOnce());
    expect([...host.querySelectorAll("button")].find(button => button.textContent === "stop send")).toBeUndefined();
    const pending = f.prepare.mock.calls[0]![0];
    await act(async () => finishPrepare({...f.frozen, requestId: pending.requestId}));
    await vi.waitFor(() => expect(f.start).toHaveBeenCalledOnce());
    await unmount(); f.history.dispose();
});

it("waits for a canceled context save to settle without starting preparation", async () => {
    const f = gateway(); localStorage.setItem(`research-assistant:independent-draft:${id}`, "Compare");
    let finishSave!: () => void;
    const saveConfiguration = vi.fn((_id: string, revision: number, state: typeof EMPTY_AGENT_CONTEXT) =>
        new Promise<{revision: number; state: typeof EMPTY_AGENT_CONTEXT}>(resolve => {finishSave = () => resolve({revision:revision+1, state});}));
    const port = {...f.port, readConfiguration: vi.fn(async () => ({revision:1, state:EMPTY_AGENT_CONTEXT})), saveConfiguration};
    const host = await mount(port);
    await vi.waitFor(() => expect([...host.querySelectorAll("button")].find(button => button.textContent === "submit")?.disabled).toBe(false));
    await click(host, "submit");
    await vi.waitFor(() => expect(saveConfiguration).toHaveBeenCalledOnce());
    await click(host, "stop send");
    expect(f.prepare).not.toHaveBeenCalled();
    expect([...host.querySelectorAll("button")].find(button => button.textContent === "stop send")?.disabled).toBe(true);
    await act(async () => finishSave());
    await vi.waitFor(() => expect(host.querySelector('[data-pending-submission="true"]')).toBeNull());
    expect(f.prepare).not.toHaveBeenCalled(); expect(f.start).not.toHaveBeenCalled();
    expect(localStorage.getItem(`research-assistant:independent-draft:${id}`)).toBe("Compare");
    await unmount(); f.history.dispose();
});

it("does not dispatch a canceled preflight after the session is unmounted", async () => {
    const f = gateway(); localStorage.setItem(`research-assistant:independent-draft:${id}`, "Compare");
    let finishCreation!: () => void;
    const ensureCreated = vi.fn(() => new Promise<void>(resolve => {finishCreation = resolve;}));
    const host = await mount(f.port, undefined, false, {pending: true, ensureCreated});
    await click(host, "submit"); await click(host, "stop send");
    await unmount(); await act(async () => finishCreation());
    expect(f.prepare).not.toHaveBeenCalled(); expect(f.start).not.toHaveBeenCalled();
    expect(localStorage.getItem(`research-assistant:independent-draft:${id}`)).toBe("Compare");
    f.history.dispose();
});

it("keeps a later edited draft when the original uncertain request is acknowledged", async () => {
    const {port,start,history}=gateway();
    const key=`research-assistant:independent-draft:${id}`;
    localStorage.setItem(key,"Compare");
    start.mockRejectedValueOnce(new Error("lost reply"));
    const host=await mount(port);
    await click(host,"submit");
    const input=host.querySelector<HTMLInputElement>("input")!;
    await act(async()=>{
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(input,"A later draft");
        input.dispatchEvent(new Event("input",{bubbles:true}));
    });
    expect(localStorage.getItem(key)).toBe("A later draft");
    await retryNotice();
    expect(start.mock.calls[0]![0].requestId).toBe(start.mock.calls[1]![0].requestId);
    expect(localStorage.getItem(key)).toBe("A later draft");
    expect(input.value).toBe("A later draft");
    await cleanup?.();cleanup=undefined;history.dispose();
});

it("returns a failed preparation to the composer and sends only after the user retries", async () => {
    const {port, prepare, start, history} = gateway();
    localStorage.setItem(`research-assistant:independent-draft:${id}`, "Compare");
    prepare.mockRejectedValueOnce(new Error("modelUnavailable"));
    vi.mocked(notify.error).mockClear();
    const host = await mount(port);
    await click(host, "submit");
    expect(notify.error).toHaveBeenCalledExactlyOnceWith(`${AGENT_PREPARATION_FAILURE}\n当前模型未配置或不支持独立对话，请重新选择。`);
    expect(host.querySelector("input")?.value).toBe("Compare");
    expect(host.querySelector<HTMLButtonElement>("button")?.disabled).toBe(false);
    expect(host.querySelector('[aria-label="对话状态"]')).toBeNull();
    expect(localStorage.getItem(`research-assistant:independent-pending:${id}`)).toBeNull();
    expect(start).not.toHaveBeenCalled();
    expect(prepare).toHaveBeenCalledOnce();
    await click(host, "submit");
    expect(prepare).toHaveBeenCalledTimes(2);
    expect(prepare.mock.calls[0]![0].requestId).not.toBe(prepare.mock.calls[1]![0].requestId);
    expect(start).toHaveBeenCalledOnce();
    await cleanup?.(); cleanup = undefined; history.dispose();
});

it.each([false,true])("a late queue acknowledgment clears only its captured persisted draft version (replaced=%s)",async replaced=>{
    const base=gateway(),draftKey=`research-assistant:independent-draft:${id}`,inputKey=`research-assistant:independent-input:${id}`;
    localStorage.setItem(draftKey,"Queued");localStorage.setItem(inputKey,"[]");
    const state={schemaVersion:1 as const,conversationId:id,revision:0,paused:false,preparingInputId:null,inputs:[]};
    let resolve!: (value:typeof state)=>void;
    const edit=vi.fn(()=>new Promise<typeof state>(done=>{resolve=done;}));
    const host=await mount({...base.port,queue:{read:async()=>state,edit}});
    await click(host,"queue");expect(edit).toHaveBeenCalledOnce();
    if(replaced)localStorage.setItem(draftKey,"Newer draft");
    await act(async()=>resolve({...state,revision:1}));
    expect(localStorage.getItem(draftKey)).toBe(replaced?"Newer draft":null);
    expect(localStorage.getItem(inputKey)).toBe(replaced?"[]":null);
});

const defaultPaper:AgentContextSelection={label:"Paper",resource:{kind:"paperGroup",resourceId:other,newMemberAccess:"readOnly",parts:[
    {resource:{kind:"paperMetadata",resourceId:other},label:"元数据",selected:true,access:"readOnly"},
    {resource:{kind:"paperNotes",resourceId:other,excludedNoteIds:[]},label:"阅读笔记",selected:true,access:"readWrite"},
    {resource:{kind:"paperPortrait",resourceId:other},label:"文献画像",selected:true,access:"readWrite"},
]}};
for(const manual of [false,true])it(`${manual?"preserves manual access in":"refreshes current defaults for"} an existing unsent draft`,async()=>{
    const {port,prepare,history}=gateway();
    observed.candidates=[defaultPaper];
    const old:AgentContextSelection={label:"Paper",resource:{kind:"paperGroup",resourceId:other,newMemberAccess:"readOnly",parts:[]}};
    const saved=selectAgentContext(EMPTY_AGENT_CONTEXT,old,"persistent");
    localStorage.setItem(contextStorageKey(id),JSON.stringify(saved));
    localStorage.setItem(`research-assistant:context-auto:${id}`,manual?"manual":agentContextKey(old.resource));
    const host=await mount(port,undefined,false,{pending:true});
    const restored=JSON.parse(localStorage.getItem(contextStorageKey(id))!);
    expect(restored).toEqual(manual?saved:selectAgentContext(saved,defaultPaper,"persistent"));
    await click(host,"submit");
    expect(prepare.mock.calls[0]?.[0].resources).toEqual(restored.persistent.map((item:AgentContextSelection)=>item.resource));
    history.dispose();
});
for (const frozenRequest of [false, true]) it(`preserves saved access for ${frozenRequest ? "a frozen request" : "an established conversation"}`, async () => {
    const {port, frozen, history} = gateway();
    observed.candidates = [defaultPaper];
    const old: AgentContextSelection = {label: "Paper", resource: {kind: "paperGroup", resourceId: other, newMemberAccess: "readOnly", parts: []}};
    const saved = selectAgentContext(EMPTY_AGENT_CONTEXT, old, "persistent");
    localStorage.setItem(contextStorageKey(id), JSON.stringify(saved));
    localStorage.setItem(`research-assistant:context-auto:${id}`, agentContextKey(old.resource));
    if (frozenRequest) localStorage.setItem(`research-assistant:independent-pending:${id}`, JSON.stringify({
        conversationId: id, requestId: frozen.requestId, contextSnapshotId: frozen.contextSnapshotId, phase: "prepared",
    }));
    await mount(port, undefined, false, {pending: frozenRequest});
    expect(JSON.parse(localStorage.getItem(contextStorageKey(id))!)).toEqual(saved);
    await cleanup?.(); cleanup = undefined; history.dispose();
});
it("names a new conversation through the shared catalog only after successful admission", async () => {
    const {port, start, history} = gateway();
    const row = {id, title: "Compare", archived: false, durableCount: 1, createdAt: "now", updatedAt: "now"};
    vi.mocked(agentCatalogGateway.rename).mockImplementation(async request => renameProof(request));
    vi.mocked(agentMetadataGateway.read).mockResolvedValue(row);
    const onRenamed = vi.fn();
    start.mockRejectedValueOnce(new Error("lost reply"));
    const host = await mount(port, undefined, false, {title: "新对话", onRenamed});
    await click(host, "submit");
    expect(agentCatalogGateway.rename).not.toHaveBeenCalled();
    await retryNotice();
    expect(agentCatalogGateway.rename).toHaveBeenCalledExactlyOnceWith({operationId: expect.any(String), id, expectedTitle: "新对话", title: "Compare"});
    expect(onRenamed).toHaveBeenCalledExactlyOnceWith(row);
    expect(localStorage.getItem(`research-assistant:independent-pending:${id}`)).toBeNull();
    await cleanup?.(); cleanup = undefined; history.dispose();
});
it("does not replace a manual title when sending the first message", async () => {
    const {port, history} = gateway();
    const host = await mount(port);
    await click(host, "submit");
    expect(agentCatalogGateway.rename).not.toHaveBeenCalled();
    await cleanup?.(); cleanup = undefined; history.dispose();
});
it("does not classify an accepted send as uncertain when title renaming conflicts", async () => {
    const {port, start, history} = gateway();
    vi.mocked(agentCatalogGateway.rename).mockRejectedValue("conflict");
    const onRenamed = vi.fn(), onRefreshTitle = vi.fn();
    const host = await mount(port, undefined, false, {title: "新对话", onRenamed, onRefreshTitle});
    await click(host, "submit");
    expect(start).toHaveBeenCalledOnce();
    expect(onRenamed).not.toHaveBeenCalled();
    expect(onRefreshTitle).toHaveBeenCalledOnce();
    expect(host.textContent).not.toContain("核对并重试同一请求");
    expect(localStorage.getItem(`research-assistant:independent-pending:${id}`)).toBeNull();
    await cleanup?.(); cleanup = undefined; history.dispose();
});
it.each(["unknown", "readFailure"])("keeps an accepted model send out of naming recovery after %s", async fault => {
    const {port, start, history} = gateway();
    if (fault === "unknown") vi.mocked(agentCatalogGateway.rename).mockRejectedValueOnce({code:"storage",reason:{code:"DATA_SQL_COMMIT_UNKNOWN"}});
    else {
        vi.mocked(agentCatalogGateway.rename).mockImplementationOnce(async request => renameProof(request));
        vi.mocked(agentMetadataGateway.read).mockRejectedValueOnce("unavailable");
    }
    const onRenamed = vi.fn();
    const host = await mount(port, undefined, false, {title:"新对话", onRenamed});
    await click(host, "submit");
    expect(start).toHaveBeenCalledTimes(1);
    expect(agentCatalogGateway.rename).toHaveBeenCalledTimes(1);
    expect(vi.mocked(agentCatalogGateway.rename).mock.calls[0]![0].operationId).not.toBe(start.mock.calls[0]![0].requestId);
    expect(onRenamed).not.toHaveBeenCalled();
    expect(localStorage.getItem(`research-assistant:independent-pending:${id}`)).toBeNull();
    expect(localStorage.getItem(`research-assistant:conversation-rename:v1:${id}`)).not.toBeNull();
    expect(host.textContent).not.toContain("核对并重试同一请求");
    await cleanup?.(); cleanup=undefined; history.dispose();
});
it("captures evidence into the draft without submitting or replacing typed text", async () => {
    const {port, prepare, start, history} = gateway();
    localStorage.setItem(`research-assistant:independent-draft:${id}`, "我的问题");
    const evidence = [{type: "annotation" as const, comment: "",source:"source://notes/note:#block{b}",title:"笔记块",text:"被选原文"}];
    const resolve = vi.fn(), reject = vi.fn();
    const host = await mount(port, {text:"默认问题",options:{mode:"draft",evidence},resources:[{kind:"note",resourceId:other}],resolve,reject});
    expect(host.querySelector("input")?.value).toBe("我的问题");
    expect(resolve).toHaveBeenCalledOnce(); expect(reject).not.toHaveBeenCalled();
    expect(prepare).not.toHaveBeenCalled(); expect(start).not.toHaveBeenCalled();
    const stored = JSON.parse(localStorage.getItem(`research-assistant:independent-input:${id}`)!);
    expect(stored[1]).toEqual(evidence[0]);
    await cleanup?.(); cleanup = undefined; history.dispose();
});
it("includes staged recommendations in the frozen message but saves them only after a confirmed receipt", async () => {
    const {port, prepare, start, history} = gateway();
    localStorage.setItem(contextStorageKey(id), JSON.stringify(EMPTY_AGENT_CONTEXT));
    localStorage.setItem(recommendationDraftKey(id), JSON.stringify([{item: source, selected: true}]));
    start.mockRejectedValueOnce(new Error("response lost"));
    const host = await mount(port);
    await click(host, "submit");
    expect(prepare.mock.calls[0]![0].resources).toEqual([source.resource]);
    expect(JSON.parse(localStorage.getItem(contextStorageKey(id))!).persistent).toEqual([]);
    expect(localStorage.getItem(recommendationDraftKey(id))).not.toBe("[]");
    await retryNotice();
    expect(start.mock.calls[0]![0].requestId).toBe(start.mock.calls[1]![0].requestId);
    expect(JSON.parse(localStorage.getItem(contextStorageKey(id))!).persistent).toEqual([source]);
    expect(localStorage.getItem(recommendationDraftKey(id))).toBe("[]");
    await cleanup?.(); cleanup = undefined; history.dispose();
});
it("retains temporary sources when preparation/start fails and consumes them only on receipt", async () => {
    const { port, prepare, start, history } = gateway();
    const initial = selectAgentContext(selectAgentContext(EMPTY_AGENT_CONTEXT, source, "temporary"), { resource: { kind: "paper", resourceId: "paper" }, label: "Paper" }, "persistent");
    localStorage.setItem(contextStorageKey(id), JSON.stringify(initial));
    localStorage.setItem(contextStorageKey(other), JSON.stringify(initial));
    prepare.mockRejectedValueOnce(new Error("prepare failed"));
    const host = await mount(port);
    await click(host, "submit");
    expect(JSON.parse(localStorage.getItem(contextStorageKey(id))!).temporary).toHaveLength(1);
    expect(start).not.toHaveBeenCalled();
    start.mockRejectedValueOnce(new Error("response lost"));
    await click(host, "submit");
    expect(prepare.mock.calls[1]![0].resources).toHaveLength(2);
    expect(start).toHaveBeenCalledOnce();
    expect(JSON.parse(localStorage.getItem(contextStorageKey(id))!).temporary).toHaveLength(1);
    await retryNotice();
    expect(start.mock.calls[0]![0].requestId).toBe(start.mock.calls[1]![0].requestId);
    expect(JSON.parse(localStorage.getItem(contextStorageKey(id))!).temporary).toEqual([]);
    expect(JSON.parse(localStorage.getItem(contextStorageKey(id))!).persistent).toHaveLength(1);
    expect(JSON.parse(localStorage.getItem(contextStorageKey(other))!).temporary).toHaveLength(1);
    await cleanup?.();
    cleanup = undefined;
    history.dispose();
});
it("blocks a new send while an uncertain receipt cannot be restored", async () => {
    const { port, frozen, prepare, history } = gateway();
    localStorage.setItem(`research-assistant:independent-pending:${id}`, JSON.stringify({ conversationId: id, requestId: frozen.requestId, contextSnapshotId: frozen.contextSnapshotId, phase: "starting" }));
    port.restore = vi.fn(async () => { throw new Error("offline"); });
    const host = await mount(port);
    expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("无法恢复待发送请求"));
    expect(host.querySelector<HTMLButtonElement>("button")!.disabled).toBe(false);
    expect(prepare).not.toHaveBeenCalled();
    await cleanup?.();
    cleanup = undefined;
    history.dispose();
});
it("allows editing after a definite pre-admission model rejection", async () => {
    const {port,start,history}=gateway();
    localStorage.setItem(contextStorageKey(id),JSON.stringify(selectAgentContext(EMPTY_AGENT_CONTEXT,source,"temporary")));
    start.mockRejectedValueOnce("modelUnavailable");
    const host=await mount(port);
    await click(host,"submit");
    expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("当前模型未配置"));
    expect(host.textContent).not.toContain("返回编辑");
    expect(localStorage.getItem(`research-assistant:independent-pending:${id}`)).toBeNull();
    expect(JSON.parse(localStorage.getItem(contextStorageKey(id))!).temporary).toHaveLength(1);
    expect(host.querySelector<HTMLButtonElement>("button")!.disabled).toBe(false);
    await cleanup?.();cleanup=undefined;history.dispose();
});
it("starts a frozen normal submission directly without disclosure or confirmation UI", async () => {
    const {port, prepare, start, history} = gateway();
    localStorage.setItem(`research-assistant:independent-draft:${id}`, "Compare");
    localStorage.setItem(contextStorageKey(id), JSON.stringify(selectAgentContext(EMPTY_AGENT_CONTEXT, source, "persistent")));
    let release!: () => void;
    const waiting = new Promise<void>(resolve => {release = resolve;});
    start.mockImplementationOnce(async frozen => {
        await waiting;
        return {schemaVersion: 1, conversationId: id, requestId: frozen.requestId, turnId: frozen.turnId, alreadyStarted: false, outcome: "completed"};
    });
    const host = await mount(port);
    await click(host, "submit");
    expect(prepare).toHaveBeenCalledOnce();
    expect(start).toHaveBeenCalledExactlyOnceWith(await prepare.mock.results[0]!.value);
    expect(JSON.parse(localStorage.getItem(`research-assistant:independent-pending:${id}`)!).phase).toBe("starting");
    expect(host.querySelector<HTMLButtonElement>("button")!.disabled).toBe(true);
    expect(localStorage.getItem(`research-assistant:independent-draft:${id}`)).toBe("Compare");
    expect(host.textContent).not.toMatch(/确认发送|返回编辑|检查本次实际内容|文本片段|deepseek-v4-pro/);
    expect(host.querySelector("details")).toBeNull();
    expect(host.querySelector(".agent-context-confirmation")).toBeNull();
    await act(async () => release());
    expect(localStorage.getItem(`research-assistant:independent-draft:${id}`)).toBeNull();
    expect(localStorage.getItem(`research-assistant:independent-pending:${id}`)).toBeNull();
    expect(start).toHaveBeenCalledOnce();
    await cleanup?.(); cleanup = undefined; history.dispose();
});
it("resumes a restored prepared request without recreating the old confirmation card", async () => {
    const {port, frozen, prepare, start, history} = gateway();
    localStorage.setItem(`research-assistant:independent-pending:${id}`, JSON.stringify({conversationId: id, requestId: frozen.requestId, contextSnapshotId: frozen.contextSnapshotId, phase: "prepared"}));
    const host = await mount(port);
    expect(start).not.toHaveBeenCalled();
    expect(host.textContent).not.toMatch(/确认发送|返回编辑|检查本次实际内容|文本片段/);
    await retryNotice();
    expect(start).toHaveBeenCalledExactlyOnceWith(frozen);
    expect(prepare).not.toHaveBeenCalled();
    await cleanup?.(); cleanup = undefined; history.dispose();
});
it("keeps the original request after an uncertain start even if its retry is rejected", async () => {
    const {port, start, history} = gateway();
    start.mockRejectedValueOnce(new Error("response lost")).mockRejectedValueOnce("modelUnavailable");
    const host = await mount(port);
    await click(host, "submit");
    expect(host.querySelector('[data-pending-submission="true"]')?.textContent).toBe("Compare");
    const receipt = localStorage.getItem(`research-assistant:independent-pending:${id}`);
    await retryNotice();
    expect(start.mock.calls[0]![0]).toEqual(start.mock.calls[1]![0]);
    expect(localStorage.getItem(`research-assistant:independent-pending:${id}`)).toBe(receipt);
    expect(host.querySelector('[data-pending-submission="true"]')?.textContent).toBe("Compare");
    expect(host.querySelector<HTMLButtonElement>("button")!.disabled).toBe(false);
    expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("发送结果尚未确认"), expect.objectContaining({action: expect.any(Object)}));
    await cleanup?.(); cleanup = undefined; history.dispose();
});

it("sends a reader request once under StrictMode using selected grants and explicit transient resources while preserving the composer draft", async () => {
    const {port, prepare, start, history} = gateway();
    const draftKey = `research-assistant:independent-draft:${id}`;
    localStorage.setItem(draftKey, "Unsent human draft");
    const initial = selectAgentContext(EMPTY_AGENT_CONTEXT, source, "temporary");
    localStorage.setItem(contextStorageKey(id), JSON.stringify(initial));
    localStorage.setItem(recommendationDraftKey(id), JSON.stringify([{item: source, selected: true}]));
    const request = {text: "Explain this exact quote", resources: [{kind:"paper" as const,resourceId:"paper",attachmentId:"pdf"}], resolve: vi.fn(), reject: vi.fn()};
    const host = await mount(port, request);
    expect(prepare).toHaveBeenCalledOnce(); expect(start).toHaveBeenCalledOnce();
    expect(prepare.mock.calls[0]![0]).toMatchObject({text: request.text, resources: [source.resource, {kind:"paper",resourceId:"paper",attachmentId:"pdf"}], userInput: []});
    expect(request.resolve).toHaveBeenCalledOnce(); expect(request.reject).not.toHaveBeenCalled();
    expect(localStorage.getItem(draftKey)).toBe("Unsent human draft");
    expect(host.querySelector("input")?.value).toBe("Unsent human draft");
    expect(JSON.parse(localStorage.getItem(contextStorageKey(id))!)).toEqual(initial);
    expect(JSON.parse(localStorage.getItem(recommendationDraftKey(id))!)).toHaveLength(1);
    await cleanup?.(); cleanup=undefined; history.dispose();
});
it("returns failed reader preparation to the selection without taking its draft", async () => {
    const {port, prepare, start, history} = gateway();
    prepare.mockRejectedValueOnce(new Error("prepare failed"));
    localStorage.setItem(`research-assistant:independent-draft:${id}`, "Saved draft");
    const request = {text: "Explain quote", resolve: vi.fn(), reject: vi.fn()};
    const host = await mount(port, request);
    expect(request.reject).toHaveBeenCalledOnce(); expect(request.resolve).not.toHaveBeenCalled();
    expect(start).not.toHaveBeenCalled();
    expect(request.reject).toHaveBeenCalledWith(new Error("prepare failed"));
    expect(host.querySelector('[role="alert"]')).toBeNull();
    expect(localStorage.getItem(`research-assistant:independent-draft:${id}`)).toBe("Saved draft");
    await cleanup?.(); cleanup=undefined; history.dispose();
});
it("keeps uncertain reader sends in chat recovery and preserves the draft on retry", async () => {
    const {port, start, history} = gateway();
    start.mockRejectedValueOnce(new Error("response lost"));
    localStorage.setItem(`research-assistant:independent-draft:${id}`, "Saved draft");
    const request = {text: "Explain quote", resolve: vi.fn(), reject: vi.fn()};
    await mount(port, request);
    expect(request.resolve).toHaveBeenCalledOnce();
    expect(JSON.parse(localStorage.getItem(`research-assistant:independent-pending:${id}`)!).preserveDraft).toBe(true);
    await retryNotice();
    expect(start.mock.calls[0]![0].requestId).toBe(start.mock.calls[1]![0].requestId);
    expect(localStorage.getItem(`research-assistant:independent-draft:${id}`)).toBe("Saved draft");
    await cleanup?.(); cleanup=undefined; history.dispose();
});
it("restores reader request recovery without clearing a human composer draft", async () => {
    const {port, frozen, start, history} = gateway();
    localStorage.setItem(`research-assistant:independent-draft:${id}`, "Saved draft");
    localStorage.setItem(`research-assistant:independent-pending:${id}`, JSON.stringify({conversationId:id, requestId:frozen.requestId, contextSnapshotId:frozen.contextSnapshotId, phase:"starting", preserveDraft:true}));
    await mount(port);
    await retryNotice();
    expect(start).toHaveBeenCalledOnce();
    expect(localStorage.getItem(`research-assistant:independent-draft:${id}`)).toBe("Saved draft");
    await cleanup?.(); cleanup=undefined; history.dispose();
});

it("rejects a reader request in archived chat without submitting", async () => {
    const {port, prepare, start, history} = gateway();
    const readConfiguration=vi.fn(async()=>null),saveConfiguration=vi.fn().mockRejectedValue("conversationUnavailable");
    const request = {text:"Explain quote", resolve:vi.fn(), reject:vi.fn()};
    await mount({...port,readConfiguration,saveConfiguration}, request, true);
    expect(request.reject).toHaveBeenCalledOnce(); expect(prepare).not.toHaveBeenCalled(); expect(start).not.toHaveBeenCalled();
    expect(readConfiguration).toHaveBeenCalledWith(id,false);expect(saveConfiguration).not.toHaveBeenCalled();
    await cleanup?.(); cleanup=undefined; history.dispose();
});
it.each(["modelUnavailable", "invalidInput"])("returns a definitive reader rejection (%s) only to the selection", async (reason) => {
    const {port, start, history} = gateway();
    start.mockRejectedValueOnce(reason);
    const request = {text:"Explain quote", resolve:vi.fn(), reject:vi.fn()};
    const host = await mount(port, request);
    expect(request.reject).toHaveBeenCalledOnce(); expect(request.resolve).not.toHaveBeenCalled();
    expect(localStorage.getItem(`research-assistant:independent-pending:${id}`)).toBeNull();
    expect(request.reject).toHaveBeenCalledWith(new Error(reason === "modelUnavailable" ? "当前模型未配置或不支持独立对话，请重新选择。" : "消息或材料选择无效，请检查后重试。"));
    expect(host.querySelector('[role="alert"]')).toBeNull();
    expect(host.querySelector<HTMLButtonElement>("button")!.disabled).toBe(false);
    await cleanup?.(); cleanup=undefined; history.dispose();
});

it("keeps a failed restored reader receipt visible when the selection no longer owns the request", async () => {
    const {port, frozen, start, history} = gateway();
    localStorage.setItem(`research-assistant:independent-pending:${id}`, JSON.stringify({conversationId:id, requestId:frozen.requestId, contextSnapshotId:frozen.contextSnapshotId, phase:"prepared", preserveDraft:true}));
    start.mockRejectedValueOnce("invalidInput");
    await mount(port);
    await retryNotice();
    expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("消息或材料选择无效"));
    expect(localStorage.getItem(`research-assistant:independent-pending:${id}`)).toBeNull();
    await cleanup?.(); cleanup=undefined; history.dispose();
});
it("sends an annotation comment as a separate question and preserves the main composer draft",async()=>{
  const {port,prepare,start,history}=gateway();
  localStorage.setItem(`research-assistant:independent-draft:${id}`,"未发送的主问题");
  const evidence=[{type:"annotation" as const,source:"source://notes/note:#block{b}",title:"原文",text:"引用",comment:"为什么？"}];
  const resolve=vi.fn(),reject=vi.fn();const host=await mount(port,{text:"为什么？",options:{evidence},resources:[source.resource],resolve,reject});
  expect(prepare).toHaveBeenCalledWith(expect.objectContaining({text:"为什么？",userInput:[expect.objectContaining({type:"text",text:"为什么？"}),evidence[0]],resources:expect.arrayContaining([source.resource])}));
  expect(start).toHaveBeenCalledOnce();expect(resolve).toHaveBeenCalledOnce();expect(reject).not.toHaveBeenCalled();
  expect(host.querySelector("input")?.value).toBe("未发送的主问题");
  await cleanup?.();cleanup=undefined;history.dispose();
});

it("captures annotations from persistent context without duplicating or downgrading its selection", async () => {
  const { port, prepare, history } = gateway();
  const persistent: AgentContextSelection = { label: "Synthetic note", resource: { kind: "noteGroup", resourceId: other,
    parts: [{ resource: source.resource, label: "Note", selected: true, access: "readWrite" }] } };
  localStorage.setItem(contextStorageKey(id), JSON.stringify(selectAgentContext(EMPTY_AGENT_CONTEXT, persistent, "persistent")));
  const resolve = vi.fn(), reject = vi.fn();
  await mount(port, { text: "为什么？", options: { mode: "draft", evidence: [
    { type: "annotation", source: "source://notes/note:#block{b}", title: "原文", text: "引用", comment: "为什么？" },
  ] }, resources: [{ kind: "noteGroup", resourceId: other, parts: [{ resource: source.resource, label: "Note", selected: true, access: "readOnly" }] }], resolve, reject });
  expect(resolve).toHaveBeenCalledOnce(); expect(reject).not.toHaveBeenCalled();
  expect(prepare).not.toHaveBeenCalled();
  const context = JSON.parse(localStorage.getItem(contextStorageKey(id))!);
  expect(context.persistent).toEqual([persistent]); expect(context.temporary).toEqual([]);
  await cleanup?.(); cleanup = undefined; history.dispose();
});
