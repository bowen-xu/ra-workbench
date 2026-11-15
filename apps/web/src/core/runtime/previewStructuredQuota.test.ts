// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { createAppRuntime } from "../../app/runtime/createAppRuntime";
import { getAppRuntime } from "./appRuntime";
import { PREVIEW_STORAGE_PREFIX, selectAppStorage } from "./appStorage";
import {createIndexedPreviewStorage} from "./previewIndexedStorage";
import {flushStorage} from "./storagePersistence";
import { ensureStructuredResourceStartup } from "./structuredResourceStartup";
import { getLiteratureGateway } from "@research-assistant/literature-library-plugin/literatureGateway";
import { isVisionByLogicAttachment } from "../../app/runtime/mock/visionByLogicFixture";

class LimitedStorage implements Storage {
  private entries = new Map<string, string>();
  limit = Infinity;
  get length() { return this.entries.size; }
  get size() { return [...this.entries].reduce((sum, [key, value]) => sum + key.length + value.length, 0); }
  key(index: number) { return [...this.entries.keys()][index] ?? null; }
  getItem(key: string) { return this.entries.get(key) ?? null; }
  setItem(key: string, value: string) {
    if (this.size - (this.entries.has(key) ? key.length + this.entries.get(key)!.length : 0) + key.length + value.length > this.limit) {
      throw new DOMException("full", "QuotaExceededError");
    }
    this.entries.set(key, value);
  }
  removeItem(key: string) { this.entries.delete(key); }
  clear() { this.entries.clear(); }
}

afterEach(() => { getAppRuntime().dispose(); selectAppStorage(window.localStorage); vi.unstubAllGlobals(); });

it("persists an uncompressed first-generation Diff and reopens it intact without rewriting old histories", async () => {
  vi.stubGlobal("fetch", vi.fn(async (url: string) => {
    if (url !== "/preview/sample.pdf") throw new Error(`unexpected network: ${url}`);
    return new Response("%PDF-1.4\nsynthetic sample");
  }));
  const backing = new LimitedStorage();
  const history = JSON.stringify({body: "保留原来的历史、证据与人工正文".repeat(60_000)});
  const initialStorage = await createIndexedPreviewStorage();
  initialStorage.setItem("old-history", history); await flushStorage(initialStorage);
  const runtime = await createAppRuntime("?preview=app", backing);
  runtime.preview!.configure({delayMs: 0, nextFailure: null});
  await ensureStructuredResourceStartup(runtime);
  const paper = (await getLiteratureGateway().list()).find(value => value.attachments.some(isVisionByLogicAttachment))!;
  const ai = runtime.gateways.getAiAssistanceGateway!;
  const resources = runtime.gateways.getStructuredResourceGateway!;
  const before = (await resources.list(paper.id)).find(value => value.kind === "structuredReading")!;
  expect(before.content).toBeNull();
  const template = (await resources.list()).find(value => value.kind === "readingTemplate" && !value.paperId)!;
  const request = {clientRequestId: crypto.randomUUID(), paperId: paper.id,
    attachmentIds: paper.attachments.map(value => value.id), overviewProfileSelectionRevision: 0, imageAssets: [],
    paperTemplateRef: {templateResourceId: template.id, templateResourceRevision: template.revision}};
  await ai.startPaperAnalysis(request);
  const saved = await resources.get(before.id);
  expect(saved.content).not.toBeNull();
  expect(saved.pendingFieldIds.length).toBeGreaterThan(0);
  expect(saved.reviewBaseline).toBeNull();
  expect(runtime.storage.getItem("old-history")).toBe(history);
  expect(backing.getItem(PREVIEW_STORAGE_PREFIX + "old-history")).toBeNull();
  for (let index = 0; index < backing.length; index += 1) {
    expect(backing.getItem(backing.key(index)!)!.startsWith("\u0000research-preview:lz16:1:")).toBe(false);
  }
  runtime.dispose();
  const reopened = await createAppRuntime("?preview=app", backing);
  reopened.preview!.configure({delayMs: 0, nextFailure: null});
  const restored = reopened.gateways.getStructuredResourceGateway!;
  expect(await restored.get(before.id)).toEqual(saved);
  const kept = await restored.review({resourceId: saved.id, expectedRevision: saved.revision,
    requestId: crypto.randomUUID(), action: "keep", scope: "all"});
  expect(kept.pendingFieldIds).toEqual([]);
  await reopened.gateways.getAiAssistanceGateway!.startPaperAnalysis({...request,
    clientRequestId: crypto.randomUUID(), regenerate: true});
  expect((await restored.get(before.id)).pendingFieldIds).toEqual([]);
  expect(reopened.storage.getItem("old-history")).toBe(history);
}, 20_000);
