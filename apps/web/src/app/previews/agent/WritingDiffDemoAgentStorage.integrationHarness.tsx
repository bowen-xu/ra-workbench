/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { EditorView } from "@codemirror/view";
import { createHash } from "node:crypto";
import { afterEach, beforeEach, expect, vi } from "vitest";
import * as desktop from "../../../core/services/agentGateway";
import { WritingEditorPane } from "../../../../../../plugins/writing_projects/src/features/writing/editor/WritingEditorPane";
import { WritingWorkspaceChangeSummary } from "../../../../../../plugins/writing_projects/src/features/writing/agent/WritingWorkspaceReview";
import { WritingDiffDemoStorage } from "./WritingDiffDemoStorage";
import { WritingDiffDemoAgentStorage } from "./WritingDiffDemoAgentStorage";
import { useWritingDiffDemoDocuments } from "./useWritingDiffDemoDocuments";
import { initialChanges, type DemoChange } from "../../../core/features/agent/WritingDiffFixtures";

const notifications = vi.hoisted(() => ({ error: vi.fn(), warning: vi.fn(), dismiss: vi.fn() }));
vi.mock("../../../core/services/notifications", () => ({ notify: notifications }));
export function notificationSpies(): Record<"error" | "warning" | "dismiss", ReturnType<typeof vi.fn>> { return notifications; }
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
export const scope = { manuscriptId: "00000000-0000-4000-8000-000000000003", conversationId: "00000000-0000-4000-8000-000000000001",
  requestId: "00000000-0000-4000-8000-000000000002", contextSnapshotId: "00000000-0000-4000-8000-000000000004" };
export const admission = { scope, turnId: "00000000-0000-4000-8000-000000000005" };
export let root: Root, host: HTMLDivElement;
export let documents: ReturnType<typeof useWritingDiffDemoDocuments>;
export let storage: WritingDiffDemoStorage, agent: WritingDiffDemoAgentStorage;
const nativeCalls: ReturnType<typeof vi.spyOn>[] = [];

beforeEach(() => {
  vi.clearAllMocks();
  vi.spyOn(crypto.subtle, "digest").mockImplementation(async (_algorithm, data) =>
    Uint8Array.from(createHash("sha256").update(new Uint8Array(data as ArrayBuffer)).digest()).buffer);
  // Real hooks, editor, persistence and historical card; only I/O is replaced.
  nativeCalls.push(vi.spyOn(desktop.agentApplicationGateway, "find"), vi.spyOn(desktop.agentApplicationGateway, "execute"),
    vi.spyOn(desktop.agentProposalGateway, "latest"), vi.spyOn(desktop.agentProposalGateway, "publish"),
    vi.spyOn(desktop.agentProposalGateway, "review"), vi.spyOn(desktop.agentProposalGateway, "readFile"),
    vi.spyOn(desktop.agentWritingFileDeliveryGateway, "list"), vi.spyOn(desktop.agentWritingFileDeliveryGateway, "acknowledge"),
    vi.spyOn(desktop.agentSessionGateway, "open"), vi.spyOn(desktop.agentSessionGateway, "readHistory"),
    vi.spyOn(desktop.agentUndoGateway, "find"), vi.spyOn(desktop.agentPendingWritingUndoGateway, "list"),
    vi.spyOn(desktop.agentPendingWritingApplicationGateway, "list"));
  storage = new WritingDiffDemoStorage(scope.manuscriptId, initialChanges);
  agent = new WritingDiffDemoAgentStorage(storage, { admission, changes: initialChanges });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove();
  agent.history.dispose();
  for (const call of nativeCalls.splice(0)) expect(call).not.toHaveBeenCalled();
  vi.restoreAllMocks();
});
export function Harness({ historyOpen = false }: { readonly historyOpen?: boolean }) {
  documents = useWritingDiffDemoDocuments(storage, scope.manuscriptId, agent);
  return <>{documents.recoveryFeedback}
    {historyOpen && <WritingWorkspaceChangeSummary scope={scope} disabled={false} fileSessions={documents.files}
      undoProposal={documents.undoProposal}
      delivery={documents.agentDelivery.entries.find(entry => entry.turnId === admission.turnId)}
      proposalGateway={agent.gateways.delivery.proposals} workspaceReviewGateway={agent.reviews} />}
    {documents.ready && <WritingEditorPane activeManuscriptId={scope.manuscriptId} activePath={documents.activePath}
      content={documents.content} openPaths={documents.openPaths} fileSessions={documents.files} saveState={documents.saveState(documents.activePath ?? "")}
      onEditorSessionReady={documents.setEditorPort} onChange={documents.onChange} onSelect={documents.select}
      onClose={() => undefined} onCursorChange={() => undefined} onCompile={() => undefined} onOpenCommands={() => undefined}
      onInsertCitation={() => undefined} onCodeCheckResult={() => undefined} codeCheckEnabled={false} hasCodeCheckErrors={false}
      completionData={{ citationKeys: [], files: initialChanges.map((change) => change.path), labels: [] }}
      preferences={{ fontSize: 14, lineWrapping: true, showLineNumbers: true, tabSize: 2 }} diagnostics={[]}
      cursorOffset={0} focusRequest={undefined} insertionRequest={undefined}
      syncActions={{ forward: () => undefined, backward: () => undefined, forwardDisabled: undefined, backwardDisabled: undefined }} />}
  </>;
}
export async function settled(assertion: () => void) {
  await vi.waitFor(async () => { await act(async () => { await Promise.resolve(); }); assertion(); });
}
export function view(): EditorView {
  const element = host.querySelector(".writing-editor-host .cm-editor")!;
  const current = EditorView.findFromDOM(element as HTMLElement);
  if (!current) throw new Error("Expected production editor");
  return current;
}
export async function mount() {
  await act(async () => root.render(<Harness />));
  await settled(() => expect(documents.files.all().map((file) => file.readOperations().length)).toEqual([1, 1, 1]));
  return view();
}
export async function journal() {
  const proposal = await agent.gateways.delivery.proposals.latest(scope);
  expect(proposal).not.toBeNull();
  const request = { manuscriptId: scope.manuscriptId, conversationId: scope.conversationId, proposalId: proposal!.proposalId };
  return { proposal: proposal!, request, state: await agent.gateways.application.applications.find(request) };
}

export function resetRoot() { root = createRoot(host); }
export function configure(changes: readonly DemoChange[]) {
  agent.history.dispose();
  storage = new WritingDiffDemoStorage(scope.manuscriptId, changes);
  agent = new WritingDiffDemoAgentStorage(storage, { admission, changes });
}
