import { describe, expect, it, vi } from "vitest";
import type { AiRun, AiTaskSpecDraft, PaperAiReadingPrepareResult } from "@research-assistant/core/ai";
import type { AiAssistanceGateway } from "../../../../../apps/web/src/core/services/aiAssistanceGateway";
import type { DoclingGateway, DoclingParseRunDetail } from "../../../../../apps/web/src/core/services/doclingGateway";
import { createPreparedPaperGenerationGateway } from "../../../../../../plugins/ra-literature-library/src/services/paperGenerationPreparation";
import { workflow } from "../../../../../../plugins/ra-literature-library/src/features/ai/paperOverviewTestFixtures";

const portrait: AiTaskSpecDraft = {taskKind: "paper_profile_classification", providerId: "fixture-generation", userGoal: "画像",
  pipelineVersion: "sources", promptVersion: "v1", outputSchemaVersion: 1, maxInputTokens: 1000, maxOutputTokens: 1000, timeoutMs: 5000,
  scopeItems: [{itemKind: "attachment", objectId: "file", parentObjectId: "paper"}]};
const reading = {clientRequestId: "request", paperId: "paper", attachmentIds: ["file"], overviewProfileSelectionRevision: 1, imageAssets: []};

function fixture(available = false) {
  const state = {available};
  const prepare = vi.fn(async (): Promise<PaperAiReadingPrepareResult> => ({
    attachmentStatuses: [{attachmentId: "file", displayName: "paper.pdf", contentSha256: "a".repeat(64), parseStatus: state.available ? "available" : "missing"}],
    contextItems: [], imageRequests: [], textItemCount: 0, tableItemCount: 0, pictureItemCount: 0,
  }));
  const parse = vi.fn(async () => { state.available = true; return {id: "parse"} as DoclingParseRunDetail; });
  const raw = {preparePaperReadingContext: prepare, startRun: vi.fn(async () => ({id: "run"} as AiRun)),
    startPaperAnalysis: vi.fn(async () => workflow("needs_review")), cancelPaperAnalysis: vi.fn(async () => false)};
  const gateway = createPreparedPaperGenerationGateway(raw as unknown as AiAssistanceGateway, () => ({parseAttachment: parse} as Pick<DoclingGateway, "parseAttachment">));
  return {state, prepare, parse, raw, gateway};
}

describe("shared production paper generation admission", () => {
  for (const stage of ["portrait", "reading"] as const) {
    it(`${stage} waits for a missing parse before admitting the task`, async () => {
      const {gateway, parse, raw, state} = fixture();
      let release = () => {};
      parse.mockImplementationOnce(() => new Promise(resolve => {release = () => {state.available = true; resolve({id: "parse"} as DoclingParseRunDetail);};}));
      const run = stage === "portrait" ? gateway.startRun(portrait) : gateway.startPaperAnalysis(reading);
      await vi.waitFor(() => expect(parse).toHaveBeenCalledOnce());
      expect(raw.startRun).not.toHaveBeenCalled(); expect(raw.startPaperAnalysis).not.toHaveBeenCalled();
      release(); await run;
      expect(stage === "portrait" ? raw.startRun : raw.startPaperAnalysis).toHaveBeenCalledOnce();
    });

    it(`${stage} rejects parsing failure before any Agent admission and can retry`, async () => {
      const {gateway, parse, raw} = fixture();
      parse.mockRejectedValueOnce(new Error("source conversion failed"));
      await expect(stage === "portrait" ? gateway.startRun(portrait) : gateway.startPaperAnalysis(reading)).rejects.toThrow("source conversion failed");
      expect(raw.startRun).not.toHaveBeenCalled(); expect(raw.startPaperAnalysis).not.toHaveBeenCalled();
      await (stage === "portrait" ? gateway.startRun(portrait) : gateway.startPaperAnalysis(reading));
      expect(stage === "portrait" ? raw.startRun : raw.startPaperAnalysis).toHaveBeenCalledOnce();
    });
  }

  it("reuses the saved parse across portrait and reading", async () => {
    const {gateway, parse} = fixture();
    await gateway.startRun(portrait); await gateway.startPaperAnalysis(reading); await gateway.startRun(portrait);
    expect(parse).toHaveBeenCalledOnce();
  });

  it("does not accept a successful conversion until the saved source is readable", async () => {
    const {gateway, parse, raw} = fixture();
    parse.mockResolvedValue({id: "not-saved"} as DoclingParseRunDetail);
    await expect(gateway.startRun(portrait)).rejects.toThrow("尚未完成结构化解析");
    expect(raw.startRun).not.toHaveBeenCalled();
  });

  it("cancels during preparation without dispatching a late reading task", async () => {
    const {gateway, parse, raw, state} = fixture();
    let release = () => {};
    parse.mockImplementationOnce(() => new Promise(resolve => {release = () => {state.available = true; resolve({id: "parse"} as DoclingParseRunDetail);};}));
    const pending = gateway.startPaperAnalysis(reading);
    const failure = expect(pending).rejects.toMatchObject({code: "canceled"});
    await vi.waitFor(() => expect(parse).toHaveBeenCalledOnce());
    expect(await gateway.cancelPaperAnalysis(reading.clientRequestId)).toBe(true);
    release(); await failure;
    expect(raw.startPaperAnalysis).not.toHaveBeenCalled(); expect(raw.cancelPaperAnalysis).not.toHaveBeenCalled();
  });

  it("does not prepare document sources for unrelated tasks", async () => {
    const {gateway, prepare, parse} = fixture();
    await gateway.startRun({...portrait, taskKind: "provider_diagnostic"});
    expect(prepare).not.toHaveBeenCalled(); expect(parse).not.toHaveBeenCalled();
  });
});
