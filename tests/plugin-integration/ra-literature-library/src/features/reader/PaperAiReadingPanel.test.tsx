/** @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type {
  ConceptRetrievalPreview,
  LlmConfiguration,
  PaperAiReadingPrepareResult,
  PaperAiReadingRegion,
  PaperAiReadingSessionDetail,
  PaperAiReadingSessionSummary,
  PaperReadingBootstrapOptions,
} from "@research-assistant/core/ai";
import type { LiteratureItem } from "@research-assistant/core/literature";
import type { ReaderCommand } from "@research-assistant/core/reader";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/services/aiAssistanceGateway";
import type { DoclingGateway } from "../../../../../../apps/web/src/core/services/doclingGateway";
import type { LiteratureGateway } from "../../../../../../../plugins/ra-literature-library/src/services/literatureGateway";
import type { ReaderCommandPayload } from "../../../../../../../plugins/ra-literature-library/src/features/reader/commands/ReaderCommandDispatcher";
import { PaperAiReadingPanel } from "./PaperAiReadingPanel";
import type {
  SpeechRecognitionErrorEventLike,
  SpeechRecognitionLike,
  SpeechRecognitionResultEventLike,
} from "../../../../../../apps/web/src/core/features/speech/speechRecognition";

const notifications = vi.hoisted(() => ({
  success: vi.fn(),
  error: vi.fn(),
  info: vi.fn(),
  warning: vi.fn(),
}));
vi.mock("../../../../../../apps/web/src/core/services/notifications", () => ({ notify: notifications }));
const tauriEvents = vi.hoisted(() => ({
  listeners: [] as Array<(event: { readonly payload: unknown }) => void>,
  listen: vi.fn(async (_event: string, callback: (event: { readonly payload: unknown }) => void) => {
    tauriEvents.listeners.push(callback);
    return vi.fn();
  }),
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  isTauri: () => true,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: tauriEvents.listen,
}));

class FakeSpeechRecognition implements SpeechRecognitionLike {
  static latest: FakeSpeechRecognition | undefined;

  continuous = false;
  interimResults = false;
  lang = "";
  maxAlternatives = 0;
  onend: (() => void) | null = null;
  onerror: ((event: SpeechRecognitionErrorEventLike) => void) | null = null;
  onresult: ((event: SpeechRecognitionResultEventLike) => void) | null = null;
  onstart: (() => void) | null = null;
  abort = vi.fn();
  start = vi.fn(() => this.onstart?.());
  stop = vi.fn(() => this.onend?.());

  constructor() {
    FakeSpeechRecognition.latest = this;
  }

  result(transcript: string): void {
    this.onresult?.({
      resultIndex: 0,
      results: { 0: { 0: { transcript }, isFinal: false, length: 1 }, length: 1 },
    });
  }
}

function item(): LiteratureItem {
  return {
    id: "paper",
    itemType: "journalArticle",
    title: "Paper",
    abstractText: "",
    containerTitle: "",
    volume: "",
    issue: "",
    pages: "",
    doi: "",
    url: "",
    language: "",
    bibtexKey: "",
    creators: [],
    tags: [],
    readingStatus: "unread",
    attachments: [{
      id: "attachment",
      displayName: "paper.pdf",
      mimeType: "application/pdf",
      byteSize: 1024,
      contentSha256: "c".repeat(64),
    }],
    projectIds: [],
    metadataSources: [],
    createdAt: "2026-08-09T00:00:00.000Z",
    updatedAt: "2026-08-09T00:00:00.000Z",
  };
}

function summary(id: string, title: string, updatedAt: string): PaperAiReadingSessionSummary {
  return {
    id,
    paperId: "paper",
    attachmentIds: ["attachment"],
    parseRunIds: ["parse"],
    providerId: "deepseek",
    modelId: "deepseek-v4-flash",
    bootstrapKind: "fresh_paper",
    conceptRetrievalDefault: false,
    title,
    status: "active",
    createdAt: "2026-08-09T00:00:00.000Z",
    updatedAt,
  };
}

function detail(id: string, title: string, message: string, answer: string): PaperAiReadingSessionDetail {
  return {
    ...summary(id, title, "2026-08-09T00:00:00.000Z"),
    turns: [{
      id: `${id}-turn`,
      sessionId: id,
      ordinal: 0,
      userMessage: message,
      assistantMessage: answer,
      messages: [
        { id: `${id}-user`, role: "user", content: message },
        ...(answer.length === 0
          ? []
          : [{ id: `${id}-assistant`, role: "assistant" as const, content: answer }]),
      ],
      providerId: "deepseek",
      modelId: "deepseek-v4-flash",
      status: "succeeded",
      inputHash: "i".repeat(64),
      outputHash: "o".repeat(64),
      inputTokens: 32,
      outputTokens: 16,
      conceptRetrievalEnabled: false,
      createdAt: "2026-08-09T00:00:00.000Z",
      finishedAt: "2026-08-09T00:00:01.000Z",
    }],
  };
}

function retrievalPreview(): ConceptRetrievalPreview {
  return {
    traceId: "trace-1",
    semanticStatus: "used",
    embeddingSpaceId: "space-1",
    indexGenerationId: "generation-1",
    seeds: [{
      conceptId: "concept-1",
      conceptRevision: 1,
      label: "Concept network",
      preferredTerm: "concept network",
      matchedBy: ["lexical", "vector"],
      lexicalRank: 1,
      vectorRank: 1,
      vectorDistance: 0.1,
      rrfScore: 0.03,
    }],
    edges: [],
    sources: [{
      anchorId: "anchor-1",
      conceptId: "concept-1",
      conceptLabel: "Concept network",
      paperId: "paper-2",
      paperTitle: "Prior paper",
      sourceKind: "definition_claim",
      sourceSubjectId: "definition-1",
      sourceSubjectRevision: 1,
      pageIndex: 2,
      quoteExact: "A source-grounded concept definition.",
      contextText: "Working definition",
      selected: true,
    }],
    selectedSourceCount: 1,
    externalPaperCount: 1,
    contextSha256: "c".repeat(64),
    expiresAt: "2026-08-11T12:15:00.000Z",
  };
}

function prepare(session?: PaperAiReadingSessionSummary): PaperAiReadingPrepareResult {
  return {
    ...(session === undefined ? {} : { session }),
    attachmentStatuses: [{
      attachmentId: "attachment",
      displayName: "paper.pdf",
      contentSha256: "c".repeat(64),
      parseRunId: "parse",
      parseStatus: "available",
    }],
    contextItems: [],
    imageRequests: [],
    textItemCount: 12,
    tableItemCount: 1,
    pictureItemCount: 0,
    model: {
      providerId: "deepseek",
      modelId: "deepseek-v4-flash",
    },
  };
}

function structuredBootstrap(): PaperReadingBootstrapOptions {
  return {
    defaultKind: "structured_history",
    structuredHistory: {
      workflowId: "workflow-1",
      workflowRevision: 3,
      workflowStatus: "needs_review",
      providerId: "qwen",
      modelId: "qwen-vl-max",
      completedTurns: ["paper_overview", "paper_concept_map"],
      attachmentCount: 1,
      updatedAt: "2026-08-09T02:00:00.000Z",
    },
  };
}

function llmConfiguration(selection = {
  providerId: "deepseek" as const,
  modelId: "deepseek-v4-flash",
}): LlmConfiguration {
  return {
    providers: [{
      providerId: "deepseek",
      displayName: "DeepSeek",
      authMode: "api-key",
      protocol: "openai-compatible",
      modelKinds: ["text", "vision"],
      defaultBaseUrls: ["https://api.deepseek.invalid"],
      baseUrl: "https://api.deepseek.invalid",
      usesDefaultBaseUrl: true,
      enabled: true,
      timeoutMs: 60_000,
      credentialConfigured: true,
    }],
    defaultModels: [{ routeKind: "default", routeId: "vision", label: "视觉模型", modelKind: "vision", selection }],
    useCases: [{ routeKind: "use_case", routeId: "paper_reading_chat", label: "论文 AI 阅读对话", modelKind: "vision", selection }],
  };
}

function button(container: HTMLElement, label: string): HTMLButtonElement {
  const match = [...container.querySelectorAll("button")]
    .find((item) => item.textContent === label || item.getAttribute("aria-label") === label);
  if (match === undefined) throw new Error(`missing button ${label}`);
  return match;
}

async function setTextareaValue(container: HTMLElement, value: string): Promise<void> {
  const textarea = container.querySelector("textarea");
  if (textarea === null) throw new Error("missing textarea");
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set?.call(textarea, value);
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function pressTextareaKey(container: HTMLElement, init: KeyboardEventInit): Promise<void> {
  const textarea = container.querySelector("textarea");
  if (textarea === null) throw new Error("missing textarea");
  await act(async () => {
    textarea.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
    await Promise.resolve();
  });
}

async function selectValue(container: HTMLElement, value: string): Promise<void> {
  const select = container.querySelector("select");
  if (select === null) throw new Error("missing select");
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")?.set?.call(select, value);
    select.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

describe("Paper AI reading sessions", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean })
      .IS_REACT_ACT_ENVIRONMENT = true;
    tauriEvents.listeners = [];
    Object.defineProperty(window, "confirm", {
      configurable: true,
      value: vi.fn(() => true),
    });
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    Reflect.deleteProperty(window, "webkitSpeechRecognition");
    FakeSpeechRecognition.latest = undefined;
    container.remove();
    vi.clearAllMocks();
  });

  async function renderPanel(aiGateway: AiAssistanceGateway, options?: {
    readonly executeReaderCommand?: (command: ReaderCommand, payload?: ReaderCommandPayload, target?: EventTarget | null) => boolean;
    readonly onHighlightContext?: (itemId: string, regions: readonly PaperAiReadingRegion[]) => void;
  }): Promise<void> {
    const defaults: Partial<AiAssistanceGateway> = {
      preparePaperReadingBootstrap: vi.fn(async () => ({
        defaultKind: "fresh_paper" as const,
        unavailableReason: "尚无可复用的结构化阅读历史",
      })),
      getLlmConfiguration: vi.fn(async () => llmConfiguration()),
      fetchLlmProviderModels: vi.fn(async () => ({
        providerId: "deepseek" as const,
        models: [{
          id: "deepseek-v4-flash",
          displayName: "DeepSeek V4 Flash",
          description: "",
          isDefault: true,
          supportedReasoningEfforts: [],
          contextWindowTokens: 128_000,
          contextWindowSource: "models_dev" as const,
        }],
      })),
      saveLlmModelRoutes: vi.fn(async () => llmConfiguration()),
    };
    const gateway = { ...defaults, ...aiGateway } as AiAssistanceGateway;
    await act(async () => root.render(<PaperAiReadingPanel
      aiGateway={gateway}
      doclingGateway={{ parseAttachment: vi.fn() } as unknown as DoclingGateway}
      executeReaderCommand={options?.executeReaderCommand ?? (() => true)}
      item={item()}
      literatureGateway={{ readAttachment: vi.fn() } as unknown as LiteratureGateway}
      onClose={() => undefined}
      {...(options?.onHighlightContext === undefined ? {} : { onHighlightContext: options.onHighlightContext })}
    />));
    await act(async () => undefined);
    await act(async () => undefined);
  }

  it("loads previous sessions and switches between them", async () => {
    const latest = summary("session-2", "Latest question", "2026-08-09T02:00:00.000Z");
    const older = summary("session-1", "Earlier question", "2026-08-09T01:00:00.000Z");
    const getPaperReadingSession = vi.fn(async (sessionId: string) =>
      sessionId === "session-1"
        ? detail("session-1", "Earlier question", "Earlier user", "Earlier answer")
        : detail("session-2", "Latest question", "Latest user", "Latest answer"));
    const getPaperReadingTurnDebugData = vi.fn(async (turnId: string) => ({
      turnId,
      rawRequest: { messages: [{ role: "system", content: "system prompt" }] },
      rawResponse: { choices: [{ message: { content: "Latest answer" } }] },
    }));
    await renderPanel({
      preparePaperReadingContext: vi.fn(async () => prepare(latest)),
      listPaperReadingSessions: vi.fn(async () => [latest, older]),
      getPaperReadingSession,
      getPaperReadingTurnDebugData,
    } as unknown as AiAssistanceGateway);

    expect(container.textContent).toContain("Latest answer");
    expect(container.textContent).toContain("Latest question");
    expect(container.textContent).toContain("查看返回数据（调试）");

    const debugDetails = container.querySelector<HTMLDetailsElement>(".paper-ai-reading-raw");
    if (debugDetails === null) throw new Error("missing debug details");
    expect(debugDetails.open).toBe(false);
    expect(getPaperReadingSession).toHaveBeenCalled();
    await act(async () => {
      debugDetails.open = true;
      debugDetails.dispatchEvent(new Event("toggle"));
      await Promise.resolve();
    });
    expect(container.textContent).not.toContain("system prompt");
    expect(getPaperReadingTurnDebugData).not.toHaveBeenCalled();
    const debugTurn = debugDetails.querySelector<HTMLButtonElement>(".paper-ai-reading-debug-record > button");
    expect(debugTurn?.getAttribute("aria-expanded")).toBe("false");
    await act(async () => {
      debugTurn?.click();
      await Promise.resolve();
    });
    expect(getPaperReadingTurnDebugData).toHaveBeenCalledExactlyOnceWith("session-2-turn");
    expect(debugTurn?.getAttribute("aria-expanded")).toBe("true");
    const debugFields = debugDetails.querySelectorAll<HTMLTextAreaElement>(".paper-ai-reading-debug-json textarea");
    expect(debugFields).toHaveLength(3);
    expect(debugFields[0]?.value).toContain("system prompt");
    expect(debugFields[1]?.value).toContain("Latest answer");
    expect(debugFields[2]?.value).toContain("\"inputTokens\": 32");

    await selectValue(container, "session-1");

    expect(getPaperReadingSession).toHaveBeenLastCalledWith("session-1");
    expect(container.textContent).toContain("Earlier answer");
  });

  it("shows raw debug records for every prior turn in the selected conversation", async () => {
    const current = detail("session-2", "Latest question", "First user", "First answer");
    const twoTurnSession: PaperAiReadingSessionDetail = {
      ...current,
      turns: [...current.turns, {
        ...current.turns[0]!,
        id: "session-2-turn-2",
        ordinal: 1,
        userMessage: "Second user",
        assistantMessage: "Second answer",
      }],
    };
    const getPaperReadingTurnDebugData = vi.fn(async (turnId: string) => ({
      turnId,
      rawRequest: { messages: [{ role: "user", content: `${turnId} request` }] },
      rawResponse: { choices: [{ message: { content: `${turnId} response` } }] },
    }));
    await renderPanel({
      preparePaperReadingContext: vi.fn(async () => prepare(twoTurnSession)),
      listPaperReadingSessions: vi.fn(async () => [twoTurnSession]),
      getPaperReadingSession: vi.fn(async () => twoTurnSession),
      getPaperReadingTurnDebugData,
    } as unknown as AiAssistanceGateway);

    const debugDetails = container.querySelectorAll<HTMLDetailsElement>(".paper-ai-reading-raw")[1];
    if (debugDetails === undefined) throw new Error("missing second debug details");
    await act(async () => {
      debugDetails.open = true;
      debugDetails.dispatchEvent(new Event("toggle"));
      await Promise.resolve();
    });

    expect(getPaperReadingTurnDebugData).not.toHaveBeenCalled();
    expect(container.textContent).toContain("会话调试时间线（截至第 2 轮）");
    expect(container.querySelectorAll(".paper-ai-reading-debug-json")).toHaveLength(0);

    const debugTurns = debugDetails.querySelectorAll<HTMLButtonElement>(".paper-ai-reading-debug-record > button");
    expect(debugTurns).toHaveLength(2);
    await act(async () => {
      debugTurns[1]?.click();
      await Promise.resolve();
    });
    expect(getPaperReadingTurnDebugData).toHaveBeenCalledExactlyOnceWith("session-2-turn-2");
    let debugFields = debugDetails.querySelectorAll<HTMLTextAreaElement>(".paper-ai-reading-debug-json textarea");
    expect(debugFields).toHaveLength(3);
    expect(debugFields[0]?.value).toContain("session-2-turn-2 request");
    expect(debugFields[1]?.value).toContain("session-2-turn-2 response");
    expect(debugFields[0]?.value).not.toContain("session-2-turn request");

    await act(async () => {
      debugTurns[0]?.click();
      await Promise.resolve();
    });
    expect(getPaperReadingTurnDebugData).toHaveBeenNthCalledWith(2, "session-2-turn");
    debugFields = debugDetails.querySelectorAll<HTMLTextAreaElement>(".paper-ai-reading-debug-json textarea");
    expect(debugFields).toHaveLength(3);
    expect(debugFields[0]?.value).toContain("session-2-turn request");
    expect(debugFields[1]?.value).not.toContain("session-2-turn-2 response");
    expect(debugTurns[0]?.getAttribute("aria-expanded")).toBe("true");
    expect(debugTurns[1]?.getAttribute("aria-expanded")).toBe("false");
  });

  it("deletes the selected session and loads the next available session", async () => {
    const latest = summary("session-2", "Latest question", "2026-08-09T02:00:00.000Z");
    const older = summary("session-1", "Earlier question", "2026-08-09T01:00:00.000Z");
    const confirm = vi.mocked(window.confirm);
    const deletePaperReadingSession = vi.fn(async () => true);
    const listPaperReadingSessions = vi.fn()
      .mockResolvedValueOnce([latest, older])
      .mockResolvedValueOnce([older]);
    const getPaperReadingSession = vi.fn(async (sessionId: string) =>
      sessionId === "session-1"
        ? detail("session-1", "Earlier question", "Earlier user", "Earlier answer")
        : detail("session-2", "Latest question", "Latest user", "Latest answer"));
    await renderPanel({
      preparePaperReadingContext: vi.fn(async () => prepare(latest)),
      listPaperReadingSessions,
      getPaperReadingSession,
      deletePaperReadingSession,
    } as unknown as AiAssistanceGateway);

    await act(async () => {
      button(container, "删除").click();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(confirm).toHaveBeenCalledWith("删除「Latest question」？该对话会从当前列表移除。");
    expect(deletePaperReadingSession).toHaveBeenCalledWith("session-2");
    expect(listPaperReadingSessions).toHaveBeenCalledTimes(2);
    expect(getPaperReadingSession).toHaveBeenLastCalledWith("session-1");
    expect(container.textContent).toContain("Earlier answer");
    expect(container.textContent).not.toContain("Latest answer");
    expect(notifications.success).toHaveBeenCalledWith("AI 阅读对话已删除");
  });

  it("starts a new session without reusing the previous session id", async () => {
    const latest = summary("session-2", "Latest question", "2026-08-09T02:00:00.000Z");
    const created = detail("session-3", "New question", "New user", "New answer");
    const startPaperReadingTurn = vi.fn(async () => created);
    const listPaperReadingSessions = vi.fn()
      .mockResolvedValueOnce([latest])
      .mockResolvedValueOnce([summary("session-3", "New question", "2026-08-09T03:00:00.000Z"), latest]);
    await renderPanel({
      preparePaperReadingContext: vi.fn(async () => prepare(latest)),
      listPaperReadingSessions,
      getPaperReadingSession: vi.fn(async (sessionId: string) =>
        sessionId === "session-3" ? created : detail("session-2", "Latest question", "Latest user", "Latest answer")),
      startPaperReadingTurn,
    } as unknown as AiAssistanceGateway);

    await act(async () => button(container, "新会话").click());
    expect(container.textContent).toContain("这是新的 AI 阅读会话");
    await setTextareaValue(container, "New user");
    await act(async () => button(container, "发送").click());

    expect(startPaperReadingTurn).toHaveBeenCalledWith(expect.not.objectContaining({
      sessionId: "session-2",
    }));
    expect(startPaperReadingTurn).toHaveBeenCalledWith(expect.objectContaining({
      paperId: "paper",
      userMessage: "New user",
      modelSelection: { providerId: "deepseek", modelId: "deepseek-v4-flash" },
    }));
    expect(container.textContent).toContain("New answer");
    expect(notifications.success).toHaveBeenCalledWith("AI 阅读回答已生成");
  });

  it("previews concept-network sources before sending them with the turn", async () => {
    const base = detail("session-3", "Question", "How does it work?", "Answer");
    const firstTurn = base.turns[0];
    if (firstTurn === undefined) throw new Error("missing turn fixture");
    const created: PaperAiReadingSessionDetail = {
      ...base,
      conceptRetrievalDefault: true,
      turns: [{
        ...firstTurn,
        conceptRetrievalEnabled: true,
        conceptRetrievalTraceId: "trace-1",
      }],
    };
    const previewConceptRetrieval = vi.fn(async () => retrievalPreview());
    const getConceptRetrievalTrace = vi.fn(async () => retrievalPreview());
    const startPaperReadingTurn = vi.fn(async () => created);
    await renderPanel({
      preparePaperReadingContext: vi.fn(async () => prepare()),
      listPaperReadingSessions: vi.fn(async () => []),
      previewConceptRetrieval,
      cancelConceptRetrievalPreview: vi.fn(async () => true),
      getConceptRetrievalTrace,
      startPaperReadingTurn,
    } as unknown as AiAssistanceGateway);

    const checkbox = container.querySelector<HTMLInputElement>(
      '.paper-concept-retrieval input[type="checkbox"]',
    );
    if (checkbox === null) throw new Error("missing retrieval checkbox");
    await act(async () => checkbox.click());
    await setTextareaValue(container, "How does it work?");
    await act(async () => {
      button(container, "发送").click();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(previewConceptRetrieval).toHaveBeenCalledWith(expect.objectContaining({
      paperId: "paper",
      query: "How does it work?",
    }));
    expect(startPaperReadingTurn).not.toHaveBeenCalled();
    expect(container.textContent).toContain("Prior paper");
    expect(container.textContent).toContain("1 篇其他论文");

    await act(async () => {
      button(container, "确认来源并发送").click();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(startPaperReadingTurn).toHaveBeenCalledWith(expect.objectContaining({
      userMessage: "How does it work?",
      conceptRetrievalTraceId: "trace-1",
    }));
    const traceSummary = [...container.querySelectorAll("summary")]
      .find((item) => item.textContent === "本轮检索上下文");
    if (traceSummary === undefined) throw new Error("missing retrieval trace summary");
    await act(async () => {
      traceSummary.click();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(getConceptRetrievalTrace).toHaveBeenCalledWith("trace-1");
    expect(container.textContent).toContain("[CN-S1] Prior paper");
  });

  it("defaults a compatible new session to the frozen structured-reading history", async () => {
    const created: PaperAiReadingSessionDetail = {
      ...detail("session-structured", "Question", "Question", "Answer"),
      bootstrapKind: "structured_history",
      sourceWorkflowId: "workflow-1",
      sourceWorkflowRevision: 3,
      bootstrapHash: "b".repeat(64),
    };
    const startPaperReadingTurn = vi.fn(async () => created);
    await renderPanel({
      preparePaperReadingContext: vi.fn(async () => prepare()),
      preparePaperReadingBootstrap: vi.fn(async () => structuredBootstrap()),
      listPaperReadingSessions: vi.fn(async () => []),
      getPaperReadingSession: vi.fn(),
      startPaperReadingTurn,
    } as unknown as AiAssistanceGateway);

    const checkbox = container.querySelector<HTMLInputElement>(
      '.paper-reading-bootstrap-control input[type="checkbox"]',
    );
    expect(checkbox?.checked).toBe(true);
    expect(container.textContent).toContain("结构化阅读、关键概念抽取");
    expect(container.textContent).toContain("包含未经人工确认的 AI 候选");

    await setTextareaValue(container, "Question");
    await act(async () => button(container, "发送").click());

    expect(startPaperReadingTurn).toHaveBeenCalledWith(expect.objectContaining({
      bootstrapKind: "structured_history",
      sourceWorkflowId: "workflow-1",
      userMessage: "Question",
    }));
    expect(container.textContent).toContain("复用结构化历史");
    expect(container.querySelector(".paper-reading-bootstrap-control")).toBeNull();
  });

  it("lets the user start fresh even when compatible structured history exists", async () => {
    const startPaperReadingTurn = vi.fn(async (
      _draft: Parameters<AiAssistanceGateway["startPaperReadingTurn"]>[0],
    ) => detail("session-fresh", "Question", "Question", "Answer"));
    await renderPanel({
      preparePaperReadingContext: vi.fn(async () => prepare()),
      preparePaperReadingBootstrap: vi.fn(async () => structuredBootstrap()),
      listPaperReadingSessions: vi.fn(async () => []),
      getPaperReadingSession: vi.fn(),
      startPaperReadingTurn,
    } as unknown as AiAssistanceGateway);

    const checkbox = container.querySelector<HTMLInputElement>(
      '.paper-reading-bootstrap-control input[type="checkbox"]',
    );
    if (checkbox === null) throw new Error("missing bootstrap checkbox");
    await act(async () => checkbox.click());
    expect(checkbox.checked).toBe(false);
    expect(container.textContent).toContain("仅发送论文内容，不包含概要和概念候选");

    await setTextareaValue(container, "Question");
    await act(async () => button(container, "发送").click());

    expect(startPaperReadingTurn).toHaveBeenCalledWith(expect.objectContaining({
      bootstrapKind: "fresh_paper",
      userMessage: "Question",
    }));
    expect(startPaperReadingTurn.mock.calls[0]?.[0]).not.toHaveProperty("sourceWorkflowId");
  });

  it("submits with Enter and keeps Shift+Enter for multiline editing", async () => {
    const created = detail("session-3", "Keyboard question", "Keyboard user", "Keyboard answer");
    const startPaperReadingTurn = vi.fn(async () => created);
    await renderPanel({
      preparePaperReadingContext: vi.fn(async () => prepare()),
      listPaperReadingSessions: vi.fn(async () => []),
      getPaperReadingSession: vi.fn(),
      startPaperReadingTurn,
    } as unknown as AiAssistanceGateway);

    await setTextareaValue(container, "Keyboard user");
    await pressTextareaKey(container, { key: "Enter", shiftKey: true });
    expect(startPaperReadingTurn).not.toHaveBeenCalled();
    await pressTextareaKey(container, { key: "Enter" });

    expect(startPaperReadingTurn).toHaveBeenCalledWith(expect.objectContaining({
      userMessage: "Keyboard user",
    }));
    expect(container.textContent).toContain("Keyboard answer");
  });

  it("streams dictated text into the composer and waits for recognition to stop before sending", async () => {
    Object.assign(window, { webkitSpeechRecognition: FakeSpeechRecognition });
    const created = detail("session-3", "Voice question", "请解释研究方法", "Voice answer");
    const startPaperReadingTurn = vi.fn(async () => created);
    await renderPanel({
      preparePaperReadingContext: vi.fn(async () => prepare()),
      listPaperReadingSessions: vi.fn(async () => []),
      getPaperReadingSession: vi.fn(),
      startPaperReadingTurn,
    } as unknown as AiAssistanceGateway);

    await act(async () => button(container, "语音输入").click());
    expect(window.confirm).toHaveBeenCalledWith(expect.stringContaining("可能使用联网识别"));
    await act(async () => FakeSpeechRecognition.latest?.result("请解释研究方法"));

    expect(container.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe("请解释研究方法");
    expect(container.textContent).toContain("正在听写");
    expect(button(container, "发送").disabled).toBe(true);
    await pressTextareaKey(container, { key: "Enter" });
    expect(startPaperReadingTurn).not.toHaveBeenCalled();

    await act(async () => button(container, "停止输入").click());
    expect(FakeSpeechRecognition.latest?.stop).toHaveBeenCalledOnce();
    expect(button(container, "发送").disabled).toBe(false);
    await act(async () => button(container, "发送").click());

    expect(startPaperReadingTurn).toHaveBeenCalledWith(expect.objectContaining({
      userMessage: "请解释研究方法",
    }));
    expect(container.textContent).toContain("Voice answer");
  });

  it("reuses stored image context on follow-up turns instead of cropping screenshots again", async () => {
    const current = detail("session-2", "Question", "First user", "First answer");
    const updated: PaperAiReadingSessionDetail = {
      ...current,
      turns: [...current.turns, {
        ...current.turns[0]!,
        id: "session-2-turn-2",
        ordinal: 1,
        userMessage: "Follow-up user",
        assistantMessage: "Follow-up answer",
      }],
    };
    const prepared: PaperAiReadingPrepareResult = {
      ...prepare(current),
      imageRequests: [{
        contextItemId: "P1-F1",
        attachmentId: "attachment",
        pageIndex: 0,
        regions: [{ pageIndex: 0, rect: { x: 0.1, y: 0.1, width: 0.4, height: 0.2 } }],
      }],
      pictureItemCount: 1,
    };
    const readAttachment = vi.fn();
    const startPaperReadingTurn = vi.fn(async () => updated);
    await act(async () => root.render(<PaperAiReadingPanel
      aiGateway={{
        getLlmConfiguration: vi.fn(async () => llmConfiguration()),
        fetchLlmProviderModels: vi.fn(async () => ({
          providerId: "deepseek" as const,
          models: [{ id: "deepseek-v4-flash", displayName: "DeepSeek V4 Flash", description: "", isDefault: true, supportedReasoningEfforts: [] }],
        })),
        saveLlmModelRoutes: vi.fn(async () => llmConfiguration()),
        preparePaperReadingContext: vi.fn(async () => prepared),
        preparePaperReadingBootstrap: vi.fn(async () => ({
          defaultKind: "fresh_paper" as const,
          unavailableReason: "尚无可复用的结构化阅读历史",
        })),
        listPaperReadingSessions: vi.fn(async () => [current]),
        getPaperReadingSession: vi.fn(async () => current),
        startPaperReadingTurn,
      } as unknown as AiAssistanceGateway}
      doclingGateway={{ parseAttachment: vi.fn() } as unknown as DoclingGateway}
      executeReaderCommand={() => true}
      item={item()}
      literatureGateway={{ readAttachment } as unknown as LiteratureGateway}
      onClose={() => undefined}
    />));
    await act(async () => undefined);
    await act(async () => undefined);

    await setTextareaValue(container, "Follow-up user");
    await act(async () => button(container, "发送").click());

    expect(readAttachment).not.toHaveBeenCalled();
    expect(startPaperReadingTurn).toHaveBeenCalledWith(expect.objectContaining({
      sessionId: "session-2",
      userMessage: "Follow-up user",
      imageAssets: [],
    }));
  });

  it("leaves Tab untouched and queues only a rejected Enter steer", async () => {
    const first = detail("session-3", "First", "First user", "First answer");
    const queued = {
      ...first,
      turns: [...first.turns, {
        ...first.turns[0]!,
        id: "session-3-turn-2",
        ordinal: 1,
        userMessage: "Queued user",
        assistantMessage: "Queued answer",
        messages: [
          { id: "queued-user", role: "user" as const, content: "Queued user" },
          { id: "queued-assistant", role: "assistant" as const, content: "Queued answer" },
        ],
      }],
    };
    let resolveFirst: (value: PaperAiReadingSessionDetail) => void = () => undefined;
    const startPaperReadingTurn = vi.fn((draft: Parameters<AiAssistanceGateway["startPaperReadingTurn"]>[0]) => {
      if (draft.userMessage === "First user") {
        return new Promise<PaperAiReadingSessionDetail>((resolve) => { resolveFirst = resolve; });
      }
      if (draft.userMessage === "Queued user") return Promise.resolve(queued);
      return Promise.reject(new Error(`unexpected message ${draft.userMessage}`));
    });
    const steerPaperReadingTurn = vi.fn<AiAssistanceGateway["steerPaperReadingTurn"]>(async () => ({
      status: "accepted" as const,
      message: { id: "steered-user", role: "user" as const, content: "Inserted user" },
    })).mockResolvedValueOnce({ status: "unsupported" as const });
    await renderPanel({
      preparePaperReadingContext: vi.fn(async () => prepare()),
      listPaperReadingSessions: vi.fn(async () => []),
      getPaperReadingSession: vi.fn(),
      startPaperReadingTurn,
      steerPaperReadingTurn,
    } as unknown as AiAssistanceGateway);

    await setTextareaValue(container, "First user");
    await act(async () => button(container, "发送").click());
    expect(container.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe("");
    expect(button(container, "中断当前回答").dataset.action).toBe("stop");
    expect(container.querySelectorAll(".paper-ai-reading-send")).toHaveLength(1);
    await setTextareaValue(container, "Queued user");
    expect(button(container, "发送").dataset.action).toBe("send");
    expect(container.querySelector('[aria-label="中断当前回答"]')).toBeNull();
    await pressTextareaKey(container, { key: "Tab" });
    expect(container.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe("Queued user");
    expect(steerPaperReadingTurn).not.toHaveBeenCalled();
    expect(startPaperReadingTurn).toHaveBeenCalledTimes(1);
    await pressTextareaKey(container, { key: "Enter" });
    expect(container.textContent).toContain("1 条待发送");
    expect(button(container, "中断当前回答").dataset.action).toBe("stop");

    await setTextareaValue(container, "Inserted user");
    await pressTextareaKey(container, { key: "Enter" });
    expect(container.textContent).toContain("1 条待发送");
    expect([...container.querySelectorAll(".paper-ai-reading-user")].map((node) => node.textContent))
      .toEqual(["First user", "Inserted user"]);
    expect(steerPaperReadingTurn).toHaveBeenCalledWith(
      startPaperReadingTurn.mock.calls[0]?.[0].clientRequestId,
      "Inserted user",
    );
    expect(startPaperReadingTurn).toHaveBeenCalledTimes(1);
    expect(container.querySelectorAll(".paper-ai-reading-send")).toHaveLength(1);
    expect(container.querySelector(".paper-ai-reading-steer")).toBeNull();

    await act(async () => {
      resolveFirst(first);
      await new Promise((resolve) => window.setTimeout(resolve, 5));
    });

    expect(startPaperReadingTurn).toHaveBeenNthCalledWith(2, expect.objectContaining({
      sessionId: "session-3",
      userMessage: "Queued user",
    }));
    expect(startPaperReadingTurn).toHaveBeenCalledTimes(2);
  });

  it("lets a queued message steer immediately or return to edit without losing a newer draft", async () => {
    let resolveFirst: (value: PaperAiReadingSessionDetail) => void = () => undefined;
    const startPaperReadingTurn = vi.fn((draft: Parameters<AiAssistanceGateway["startPaperReadingTurn"]>[0]) => {
      if (draft.userMessage === "First user") {
        return new Promise<PaperAiReadingSessionDetail>((resolve) => { resolveFirst = resolve; });
      }
      return Promise.resolve(detail("session-4", draft.userMessage, draft.userMessage, "Answer"));
    });
    const steerPaperReadingTurn = vi.fn<AiAssistanceGateway["steerPaperReadingTurn"]>(async (_requestId: string, userMessage: string) => ({
      status: "accepted" as const,
      message: { id: `steered-${userMessage}`, role: "user" as const, content: userMessage },
    })).mockResolvedValueOnce({ status: "unsupported" as const })
      .mockResolvedValueOnce({ status: "unsupported" as const });
    await renderPanel({
      preparePaperReadingContext: vi.fn(async () => prepare()),
      listPaperReadingSessions: vi.fn(async () => []),
      getPaperReadingSession: vi.fn(),
      startPaperReadingTurn,
      steerPaperReadingTurn,
    } as unknown as AiAssistanceGateway);

    await setTextareaValue(container, "First user");
    await act(async () => button(container, "发送").click());
    await setTextareaValue(container, "Queued A");
    await pressTextareaKey(container, { key: "Enter" });
    await setTextareaValue(container, "Queued B");
    await pressTextareaKey(container, { key: "Enter" });

    await act(async () => button(container, "插入当前回答：Queued B").click());
    const queuedLabels = [...container.querySelectorAll(".paper-ai-reading-followups li p")]
      .map((element) => element.textContent);
    expect(queuedLabels).toEqual(["Queued A"]);
    expect([...container.querySelectorAll(".paper-ai-reading-user")].map((node) => node.textContent))
      .toEqual(["First user", "Queued B"]);
    expect(steerPaperReadingTurn).toHaveBeenCalledWith(
      startPaperReadingTurn.mock.calls[0]?.[0].clientRequestId,
      "Queued B",
    );

    await setTextareaValue(container, "Newer draft");
    await act(async () => button(container, "返回编辑待发送消息：Queued A").click());
    expect(container.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe("Queued A");
    expect(container.textContent).toContain("Newer draft");

    const queueToggle = button(container, "1 条待发送");
    await act(async () => queueToggle.click());
    expect(queueToggle.getAttribute("aria-expanded")).toBe("false");

    await act(async () => {
      resolveFirst(detail("session-4", "First", "First user", "First answer"));
      await new Promise((resolve) => window.setTimeout(resolve, 5));
    });
    expect(startPaperReadingTurn).toHaveBeenNthCalledWith(2, expect.objectContaining({
      userMessage: "Newer draft",
    }));
  });

  it("returns a failed steer to the next-turn queue without faking a conversation message", async () => {
    let resolveFirst: (value: PaperAiReadingSessionDetail) => void = () => undefined;
    const startPaperReadingTurn = vi.fn((draft: Parameters<AiAssistanceGateway["startPaperReadingTurn"]>[0]) => {
      if (draft.userMessage === "First user") {
        return new Promise<PaperAiReadingSessionDetail>((resolve) => { resolveFirst = resolve; });
      }
      return Promise.reject(new Error("Steer failed"));
    });
    await renderPanel({
      preparePaperReadingContext: vi.fn(async () => prepare()),
      listPaperReadingSessions: vi.fn(async () => []),
      getPaperReadingSession: vi.fn(),
      startPaperReadingTurn,
      steerPaperReadingTurn: vi.fn(async () => { throw new Error("Steer failed"); }),
    } as unknown as AiAssistanceGateway);

    await setTextareaValue(container, "First user");
    await act(async () => button(container, "发送").click());
    await setTextareaValue(container, "Steered user");
    await pressTextareaKey(container, { key: "Enter" });

    await act(async () => Promise.resolve());
    expect(container.textContent).toContain("1 条待发送");
    expect([...container.querySelectorAll(".paper-ai-reading-user")].map((node) => node.textContent))
      .toEqual(["First user"]);

    await act(async () => {
      resolveFirst(detail("session-steer", "First", "First user", "First answer"));
      await new Promise((resolve) => window.setTimeout(resolve, 5));
    });

    expect(startPaperReadingTurn).toHaveBeenNthCalledWith(2, expect.objectContaining({
      userMessage: "Steered user",
    }));
  });

  it("interrupts the active provider turn and keeps queued follow-ups", async () => {
    const canceledSession: PaperAiReadingSessionDetail = {
      ...detail("session-5", "Canceled", "First user", ""),
      turns: [{
        ...detail("session-5", "Canceled", "First user", "").turns[0]!,
        status: "canceled",
        errorCode: "canceled",
        errorMessage: "AI 运行已取消",
      }],
    };
    let rejectTurn: (reason: unknown) => void = () => undefined;
    const startPaperReadingTurn = vi.fn((() => new Promise<PaperAiReadingSessionDetail>((_resolve, reject) => {
      rejectTurn = reject;
    })) as AiAssistanceGateway["startPaperReadingTurn"]);
    const cancelPaperReadingTurn = vi.fn(async () => {
      rejectTurn(new Error("AI 运行已取消"));
      return {
        accepted: true,
        resubmitMessages: [{
          id: "pending-steer",
          role: "user" as const,
          content: "Steered user",
        }],
      };
    });
    const listPaperReadingSessions = vi.fn()
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([canceledSession]);
    await renderPanel({
      preparePaperReadingContext: vi.fn(async () => prepare()),
      listPaperReadingSessions,
      getPaperReadingSession: vi.fn(async () => canceledSession),
      startPaperReadingTurn,
      cancelPaperReadingTurn,
      steerPaperReadingTurn: vi.fn(async () => ({status: "unsupported" as const})),
    } as unknown as AiAssistanceGateway);

    await setTextareaValue(container, "First user");
    await act(async () => button(container, "发送").click());
    await setTextareaValue(container, "Keep queued");
    await pressTextareaKey(container, { key: "Enter" });
    expect(container.textContent).toContain("Keep queued");

    await pressTextareaKey(container, { key: "Escape" });
    await act(async () => Promise.resolve());

    expect(cancelPaperReadingTurn).toHaveBeenCalledWith(
      startPaperReadingTurn.mock.calls[0]?.[0].clientRequestId,
    );
    expect([...container.querySelectorAll(".paper-ai-reading-followups li p")]
      .map((element) => element.textContent)).toEqual(["Steered user", "Keep queued"]);
    expect(container.textContent).toContain("Keep queued");
    expect(container.textContent).toContain("AI 运行已取消");
  });

  it("keeps the turn active when the provider has not accepted cancellation", async () => {
    let resolveTurn: (value: PaperAiReadingSessionDetail) => void = () => undefined;
    const startPaperReadingTurn = vi.fn((() => new Promise<PaperAiReadingSessionDetail>((resolve) => {
      resolveTurn = resolve;
    })) as AiAssistanceGateway["startPaperReadingTurn"]);
    const cancelPaperReadingTurn = vi.fn(async () => ({ accepted: false, resubmitMessages: [] }));
    await renderPanel({
      preparePaperReadingContext: vi.fn(async () => prepare()),
      listPaperReadingSessions: vi.fn(async () => []),
      getPaperReadingSession: vi.fn(),
      startPaperReadingTurn,
      cancelPaperReadingTurn,
    } as unknown as AiAssistanceGateway);

    await setTextareaValue(container, "First user");
    await act(async () => button(container, "发送").click());
    await act(async () => {
      button(container, "中断当前回答").click();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(container.textContent).toContain("无法中断当前回答");
    expect(button(container, "中断当前回答").disabled).toBe(false);

    await act(async () => resolveTurn(detail("session-6", "Done", "First user", "Done")));
    expect(container.textContent).toContain("Done");
  });

  it("can hide the panel while an answer continues", async () => {
    let resolveTurn: (value: PaperAiReadingSessionDetail) => void = () => undefined;
    const onClose = vi.fn();
    await act(async () => root.render(<PaperAiReadingPanel
      aiGateway={{
        getLlmConfiguration: vi.fn(async () => llmConfiguration()),
        fetchLlmProviderModels: vi.fn(async () => ({
          providerId: "deepseek" as const,
          models: [{ id: "deepseek-v4-flash", displayName: "DeepSeek V4 Flash", description: "", isDefault: true, supportedReasoningEfforts: [] }],
        })),
        saveLlmModelRoutes: vi.fn(async () => llmConfiguration()),
        preparePaperReadingContext: vi.fn(async () => prepare()),
        preparePaperReadingBootstrap: vi.fn(async () => ({
          defaultKind: "fresh_paper" as const,
          unavailableReason: "尚无可复用的结构化阅读历史",
        })),
        listPaperReadingSessions: vi.fn(async () => []),
        getPaperReadingSession: vi.fn(),
        startPaperReadingTurn: vi.fn(() => new Promise((resolve) => { resolveTurn = resolve; })),
      } as unknown as AiAssistanceGateway}
      doclingGateway={{ parseAttachment: vi.fn() } as unknown as DoclingGateway}
      executeReaderCommand={() => true}
      item={item()}
      literatureGateway={{ readAttachment: vi.fn() } as unknown as LiteratureGateway}
      onClose={onClose}
    />));
    await act(async () => undefined);
    await act(async () => undefined);

    await setTextareaValue(container, "Continue in background");
    await act(async () => button(container, "发送").click());
    await act(async () => button(container, "隐藏 AI 阅读（回答继续生成）").click());
    expect(onClose).toHaveBeenCalledOnce();

    await act(async () => resolveTurn(detail("session-6", "Done", "Continue in background", "Done")));
  });

  it("renders streaming answer deltas before the turn command resolves", async () => {
    const created = detail("session-3", "Streaming question", "Stream user", "Final answer");
    let resolveTurn: (value: PaperAiReadingSessionDetail) => void = () => undefined;
    const startPaperReadingTurn = vi.fn((async () =>
      new Promise<PaperAiReadingSessionDetail>((resolve) => { resolveTurn = resolve; })) as AiAssistanceGateway["startPaperReadingTurn"]);
    await renderPanel({
      preparePaperReadingContext: vi.fn(async () => prepare()),
      listPaperReadingSessions: vi.fn(async () => []),
      getPaperReadingSession: vi.fn(),
      startPaperReadingTurn,
    } as unknown as AiAssistanceGateway);

    await setTextareaValue(container, "Stream user");
    await act(async () => button(container, "发送").click());
    expect(button(container, "选择 AI 文献阅读模型").disabled).toBe(true);

    const request = startPaperReadingTurn.mock.calls[0]?.[0];
    expect(request?.clientRequestId).toEqual(expect.any(String));
    await act(async () => {
      tauriEvents.listeners[0]?.({
        payload: {
          clientRequestId: request?.clientRequestId,
          paperId: "paper",
          sessionId: "session-3",
          turnId: "turn-stream",
          ordinal: 0,
          status: "delta",
          messageId: "stream-assistant",
          delta: "Partial ",
          accumulated: "Partial ",
        },
      });
      tauriEvents.listeners[0]?.({
        payload: {
          clientRequestId: request?.clientRequestId,
          paperId: "paper",
          sessionId: "session-3",
          turnId: "turn-stream",
          ordinal: 0,
          status: "delta",
          messageId: "stream-assistant",
          delta: "answer [P1-T1]",
          accumulated: "Partial answer [P1-T1]",
        },
      });
      await new Promise((resolve) => window.setTimeout(resolve, 30));
    });

    expect(container.textContent).toContain("Partial answer");
    expect(container.textContent).toContain("查看流式数据");
    expect(container.textContent).toContain("transient_stream_event_only");

    await act(async () => resolveTurn(created));
    expect(container.textContent).toContain("Final answer");
    expect(container.textContent).not.toContain("Partial answer");
  });

  it("navigates to a cited structure block and requests a transient region highlight", async () => {
    const current = detail("session-2", "Question", "User", "See [P1-T3].");
    const prepared: PaperAiReadingPrepareResult = {
      ...prepare(current),
      contextItems: [{
        id: "P1-T3",
        paperId: "paper",
        attachmentId: "attachment",
        contentSha256: "c".repeat(64),
        parseRunId: "parse",
        rawRef: "#/texts/2",
        kind: "text",
        pageIndex: 2,
        regions: [{ pageIndex: 2, rect: { x: .1, y: .2, width: .4, height: .08 } }],
        text: "Source text",
      }],
    };
    const executeReaderCommand = vi.fn(() => true);
    const onHighlightContext = vi.fn();
    await renderPanel({
      preparePaperReadingContext: vi.fn(async () => prepared),
      listPaperReadingSessions: vi.fn(async () => [current]),
      getPaperReadingSession: vi.fn(async () => current),
    } as unknown as AiAssistanceGateway, { executeReaderCommand, onHighlightContext });

    const citation = container.querySelector<HTMLButtonElement>(".paper-ai-reading-citation");
    if (citation === null) throw new Error("missing citation button");
    await act(async () => citation.click());

    expect(executeReaderCommand).toHaveBeenCalledWith("reader.navigate", { pageIndex: 2 });
    expect(onHighlightContext).toHaveBeenCalledWith("P1-T3", prepared.contextItems[0]?.regions);
  });

  it("shows real context usage and model provenance", async () => {
    const base = detail("session-2", "Question", "User", "Answer");
    const session: PaperAiReadingSessionDetail = {
      ...base,
      turns: [{
        ...base.turns[0]!,
        reasoningEffort: "high",
        inputTokens: 96_000,
        contextWindowTokens: 120_000,
        contextWindowSource: "models_dev",
      }],
    };
    await renderPanel({
      preparePaperReadingContext: vi.fn(async () => prepare(session)),
      listPaperReadingSessions: vi.fn(async () => [session]),
      getPaperReadingSession: vi.fn(async () => session),
    } as unknown as AiAssistanceGateway);

    await act(async () => button(container, "查看上下文窗口用量").click());
    expect(document.body.textContent).toContain("上一轮输入：96,000 tokens");
    expect(document.body.textContent).toContain("已用 80%");
    expect(document.body.textContent).toContain("deepseek · deepseek-v4-flash");
    expect(document.body.textContent).toContain("推理强度：high");
  });

  it("only persists the composer model after the explicit default action", async () => {
    const saveLlmModelRoutes = vi.fn(async () => llmConfiguration());
    await renderPanel({
      preparePaperReadingContext: vi.fn(async () => prepare()),
      listPaperReadingSessions: vi.fn(async () => []),
      getLlmConfiguration: vi.fn(async () => llmConfiguration()),
      fetchLlmProviderModels: vi.fn(async () => ({
        providerId: "deepseek" as const,
        models: [
          { id: "deepseek-v4-flash", displayName: "DeepSeek V4 Flash", description: "", isDefault: true, supportedReasoningEfforts: [] },
          { id: "deepseek-v4-research", displayName: "DeepSeek V4 Research", description: "", isDefault: false, supportedReasoningEfforts: [] },
        ],
      })),
      saveLlmModelRoutes,
    } as unknown as AiAssistanceGateway);

    await act(async () => button(container, "选择 AI 文献阅读模型").click());
    await act(async () => button(document.body, "选择模型").click());
    const modelMenu = document.body.querySelector('[aria-label="model 选项"]');
    expect(modelMenu?.textContent).toContain("DeepSeek V4 Research");
    expect(modelMenu?.textContent).not.toContain("deepseek-v4-research");
    await act(async () => button(document.body, "选择模型 DeepSeek V4 Research").click());
    expect(saveLlmModelRoutes).not.toHaveBeenCalled();

    await act(async () => button(document.body, "设为文献阅读默认").click());
    expect(saveLlmModelRoutes).toHaveBeenCalledWith([{
      routeKind: "use_case",
      routeId: "paper_reading_chat",
      selection: { providerId: "deepseek", modelId: "deepseek-v4-research" },
    }]);

    await act(async () => button(document.body, "恢复全局视觉默认").click());
    expect(saveLlmModelRoutes).toHaveBeenLastCalledWith([{
      routeKind: "use_case",
      routeId: "paper_reading_chat",
    }]);
  });

  it("restores the model trigger focus after Escape removes the portal", async () => {
    await renderPanel({
      preparePaperReadingContext: vi.fn(async () => prepare()),
      listPaperReadingSessions: vi.fn(async () => []),
      getLlmConfiguration: vi.fn(async () => llmConfiguration()),
    } as unknown as AiAssistanceGateway);

    const trigger = button(container, "选择 AI 文献阅读模型");
    await act(async () => trigger.click());
    expect(document.body.querySelector('[aria-label="AI 文献阅读模型"]')).not.toBeNull();
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Escape" }));
      await new Promise<void>((resolve) => window.requestAnimationFrame(() => resolve()));
    });

    expect(document.body.querySelector('[aria-label="AI 文献阅读模型"]')).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it("switches models for a later turn without changing the session or feature default", async () => {
    const current = detail("session-2", "Question", "First user", "First answer");
    const updated: PaperAiReadingSessionDetail = {
      ...current,
      modelId: "deepseek-v4-research",
      turns: [...current.turns, {
        ...current.turns[0]!,
        id: "session-2-turn-2",
        ordinal: 1,
        userMessage: "Second user",
        assistantMessage: "Second answer",
        messages: [
          { id: "second-user", role: "user", content: "Second user" },
          { id: "second-assistant", role: "assistant", content: "Second answer" },
        ],
        modelId: "deepseek-v4-research",
      }],
    };
    const startPaperReadingTurn = vi.fn(async () => updated);
    const saveLlmModelRoutes = vi.fn(async () => llmConfiguration());
    await renderPanel({
      preparePaperReadingContext: vi.fn(async () => prepare(current)),
      listPaperReadingSessions: vi.fn(async () => [current]),
      getPaperReadingSession: vi.fn(async () => current),
      startPaperReadingTurn,
      fetchLlmProviderModels: vi.fn(async () => ({
        providerId: "deepseek" as const,
        models: [
          { id: "deepseek-v4-flash", displayName: "DeepSeek V4 Flash", description: "", isDefault: true, supportedReasoningEfforts: [] },
          { id: "deepseek-v4-research", displayName: "DeepSeek V4 Research", description: "", isDefault: false, supportedReasoningEfforts: [] },
        ],
      })),
      saveLlmModelRoutes,
    } as unknown as AiAssistanceGateway);

    await act(async () => button(container, "选择 AI 文献阅读模型").click());
    await act(async () => button(document.body, "选择模型").click());
    await act(async () => button(document.body, "选择模型 DeepSeek V4 Research").click());
    await act(async () => document.body.dispatchEvent(new Event("pointerdown", { bubbles: true })));
    await setTextareaValue(container, "Second user");
    await act(async () => button(container, "发送").click());

    expect(startPaperReadingTurn).toHaveBeenCalledWith(expect.objectContaining({
      sessionId: "session-2",
      userMessage: "Second user",
      modelSelection: { providerId: "deepseek", modelId: "deepseek-v4-research" },
    }));
    expect(saveLlmModelRoutes).not.toHaveBeenCalled();
    expect(container.textContent).toContain("Second answer");
    expect(button(container, "当前环境不支持语音输入").disabled).toBe(true);
  });
});
