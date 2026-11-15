/** @vitest-environment happy-dom */
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect, it, vi} from "vitest";
import {useConversationRetrieval} from "./useConversationRetrieval";

const gateway = vi.hoisted(() => ({preview: vi.fn(), cancel: vi.fn()}));
vi.mock("../../../services/aiAssistanceGateway", () => ({getAiAssistanceGateway: () => ({previewConceptRetrieval: gateway.preview, cancelConceptRetrievalPreview: gateway.cancel})}));
vi.mock("../presentation/PaperConceptRetrievalControl", () => ({PaperConceptRetrievalControl: (props: {onCancel:()=>void; onConfirm:()=>void}) => <>
  <button onClick={props.onCancel}>cancel retrieval</button><button onClick={props.onConfirm}>confirm retrieval</button>
</>}));

it("retains the original submission identity through retrieval confirmation and drops it on cancellation", async () => {
  gateway.preview.mockResolvedValue({traceId:"retrieval-trace"}); gateway.cancel.mockResolvedValue(true);
  const confirmed = vi.fn();
  let retrieval!: ReturnType<typeof useConversationRetrieval>;
  function Harness() {retrieval = useConversationRetrieval(confirmed); return <>{retrieval.controls}</>;}
  const element = document.createElement("div"); document.body.append(element); const root = createRoot(element);
  try {
    await act(async () => root.render(<Harness/>));
    const options = {contextScope:"manuscript" as const};
    await act(async () => retrieval.search("question", options, "original-request"));
    await act(async () => (element.querySelectorAll("button")[1] as HTMLButtonElement).click());
    expect(confirmed).toHaveBeenCalledExactlyOnceWith("question", options, "retrieval-trace", "original-request");
    await act(async () => retrieval.search("another", options, "canceled-request"));
    await act(async () => (element.querySelectorAll("button")[0] as HTMLButtonElement).click());
    expect(confirmed).toHaveBeenCalledOnce();
    expect(retrieval.pending).toBe(false);
  } finally {await act(async () => root.unmount()); element.remove();}
});
