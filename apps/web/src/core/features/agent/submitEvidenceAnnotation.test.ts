import { expect, it, vi } from "vitest";
import type { AgentEvidenceInput, AgentContextResource } from "@research-assistant/core/agent";
import { formatSourceLink } from "@research-assistant/core/source";
import { submitEvidenceAnnotation } from "./submitEvidenceAnnotation";
import { releaseRemovedEvidence } from "./evidenceImageGateway";
vi.mock("./evidenceImageGateway", () => ({ releaseRemovedEvidence: vi.fn() }));
const captured: readonly AgentEvidenceInput[] = [{ type: "annotation", title: "笔记选区", comment: "",
  text: "Selected note text", source: formatSourceLink({ path: ["notes", "page"], locator: {
    kind: "block", blockId: "paragraph", range: { startLine: 1, startColumn: 1, endLine: 1, endColumn: 18 },
  } }) }];
const resources: readonly AgentContextResource[] = [{ kind: "noteGroup", resourceId: "page", parts: [{
  resource: { kind: "note", resourceId: "page" }, selected: true, access: "readOnly", label: "引用的笔记",
}] }];
it("shares Add, Send and silent-dismiss behavior with immutable captured note evidence", async () => {
  const submit = vi.fn(async () => undefined), discard = vi.fn();
  const options = { capture: async () => captured, resources, comment: "Why?", submit, discard };
  await submitEvidenceAnnotation(options);
  expect(submit).toHaveBeenLastCalledWith("", resources, { mode: "draft", evidence: [{ ...captured[0], comment: "Why?" }] });
  await submitEvidenceAnnotation({ ...options, intent: "silent" });
  expect(submit).toHaveBeenLastCalledWith("", resources, { mode: "draft", reveal: false, evidence: [{ ...captured[0], comment: "Why?" }] });
  await submitEvidenceAnnotation({ ...options, intent: "send" });
  expect(submit).toHaveBeenLastCalledWith("Why?", resources, { evidence: [{ ...captured[0], comment: "Why?" }] });
  expect(discard).toHaveBeenCalledTimes(3); expect(captured[0]?.comment).toBe("");
});
it("preserves a failed capture for retry and rejects empty questions before capture", async () => {
  const capture = vi.fn(async () => captured), submit = vi.fn(async () => { throw new Error("Busy"); }), discard = vi.fn();
  const options = { capture, resources, comment: "Why?", submit, discard };
  await expect(submitEvidenceAnnotation(options)).rejects.toThrow("Busy");
  expect(discard).not.toHaveBeenCalled(); expect(releaseRemovedEvidence).toHaveBeenCalled();
  capture.mockClear(); submit.mockClear();
  await expect(submitEvidenceAnnotation({ ...options, comment: " ", intent: "send" })).rejects.toThrow("请输入");
  expect(capture).not.toHaveBeenCalled(); expect(submit).not.toHaveBeenCalled();
});
it("does not submit an empty selection", async () => {
  const submit = vi.fn(async () => undefined), discard = vi.fn();
  await expect(submitEvidenceAnnotation({ capture: async () => [], resources, comment: "", submit, discard })).rejects.toThrow("选择内容");
  expect(submit).not.toHaveBeenCalled(); expect(discard).not.toHaveBeenCalled();
});
