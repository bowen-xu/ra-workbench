/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { AgentDraftProvider, useAgentDraftStore, useConversationDraft } from "../../../../../../apps/web/src/core/features/agent/AgentDraftProvider";
import { AgentSteeringProvider } from "../../../../../../apps/web/src/core/features/agent/AgentSteeringProvider";
import { usePaperSteering } from "./usePaperSteering";

const steer = vi.hoisted(() => vi.fn());
vi.mock("../../../../../../apps/web/src/core/services/agentGateway", () => ({ agentPaperSubmissionGateway: { steer } }));
vi.mock("../../../../../../apps/web/src/core/services/agentContextGateway", () => ({ agentContextGateway: {} }));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const conversationId = "11111111-1111-4111-8111-111111111111";
const paperId = "paper-1";
const turnId = "22222222-2222-4222-8222-222222222222";

it("reattaches paper steering after the panel is hidden and retries the original request", async () => {
  let current!: ReturnType<typeof usePaperSteering>;
  let draft!: ReturnType<typeof useConversationDraft>;
  const root = createRoot(document.createElement("div"));
  function Host() {
    draft = useConversationDraft(useAgentDraftStore(), { hostKind: "paper", hostId: paperId, conversationId });
    current = usePaperSteering(
      { conversationId, paperId, turnId },
      () => undefined,
      null,
      { key: JSON.stringify(["paper", paperId, conversationId]), conversationId, captureAcknowledgment: draft[2] },
    );
    return null;
  }
  const render = async (present: boolean) => act(async () => root.render(<AgentSteeringProvider><AgentDraftProvider>{present && <Host />}</AgentDraftProvider></AgentSteeringProvider>));
  let reject!: (reason: unknown) => void;
  steer.mockReset().mockImplementationOnce(() => new Promise((_resolve, fail) => { reject = fail; })).mockResolvedValueOnce({});
  try {
    await render(true);
    await act(async () => draft[1]("original"));
    await act(async () => { void current.submit("original"); });
    await render(false);
    await act(async () => { reject(new Error("lost response")); await Promise.resolve(); });
    await render(true);
    expect(current.uncertain).toBe(true);
    expect(draft[0]).toBe("original");
    const firstInputId = steer.mock.calls[0]?.[0].inputId;
    await act(async () => { await current.retry(); });
    expect(steer.mock.calls[1]?.[0]).toEqual(expect.objectContaining({ inputId: firstInputId, text: "original", paperId }));
    expect(draft[0]).toBe("");
  } finally {
    await act(async () => root.unmount());
  }
});
