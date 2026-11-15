import { describe, expect, it } from "vitest";

import {
  decodeAiCredentialSummary,
  decodeAiProviderOverview,
  decodeAiRun,
  decodeAiTaskPreview,
  decodeLlmConfiguration,
  decodeLlmProviderModels,
  decodePaperAiReadingCancelResult,
  decodePaperAiReadingSessionDetail,
  decodePaperAiReadingSteerResult,
  decodePaperAiReadingTurnDebugData,
  decodePaperAnalysisWorkflow,
} from "./aiDecoders";

describe("AI IPC decoders", () => {
  it("decodes a two-turn paper analysis workflow and rejects unknown states", () => {
    const attempt = {
      id: "attempt", turnId: "overview-turn", attempt: 1, status: "succeeded",
      normalizedMessages: [{ kind: "task_prompt", content: "Extract overview" }],
      providerId: "qwen", modelId: "qwen-vl", aiRunId: "run",
      candidateId: "candidate", inputHash: "a".repeat(64), outputHash: "b".repeat(64),
      inputTokens: 100, outputTokens: 20, createdAt: "now", startedAt: "now",
      finishedAt: "later", recoveryReason: "local_recovery", recoveryVersion: 1,
      recoveredAt: "later",
    };
    const workflow = {
      id: "workflow", paperId: "paper", contextSnapshotId: "snapshot",
      status: "partial", providerId: "qwen", modelId: "qwen-vl",
      promptSetVersion: "paper-analysis-v1", revision: 4, createdAt: "now",
      updatedAt: "later", completedAt: "later",
      turns: [
        { id: "overview-turn", workflowId: "workflow", ordinal: 0,
          kind: "paper_overview", status: "succeeded", candidateId: "candidate",
          createdAt: "now", finishedAt: "later", attempts: [attempt] },
        { id: "concept-turn", workflowId: "workflow", ordinal: 1,
          kind: "paper_concept_map", status: "failed", createdAt: "now",
          finishedAt: "later", attempts: [{ ...attempt, id: "concept-attempt",
            turnId: "concept-turn", status: "failed", aiRunId: undefined,
            candidateId: undefined, inputHash: undefined, outputHash: undefined,
            inputTokens: undefined, outputTokens: undefined, errorCode: "invalid_response",
            errorMessage: "provider response invalid" }] },
      ],
    };
    const decoded = decodePaperAnalysisWorkflow(workflow);
    expect(decodePaperAnalysisWorkflow({...workflow, conversationId: "shared-reading"})?.conversationId).toBe("shared-reading");
    expect(decodePaperAnalysisWorkflow({...workflow, conversationId: 123})).toBeUndefined();
    expect(decoded?.status).toBe("partial");
    expect(decoded?.turns[0]?.attempts[0]?.recoveryVersion).toBe(1);
    expect(decodePaperAnalysisWorkflow({ ...workflow, status: "invented" })).toBeUndefined();
    const singleStage = { ...workflow, status: "overview_running", activeClientRequestId: "request",
      turns: [{ ...workflow.turns[0], status: "running" }] };
    expect(decodePaperAnalysisWorkflow(singleStage)).toMatchObject({
      status: "overview_running", activeClientRequestId: "request", turns: [{ kind: "paper_overview" }],
    });
    for (const turns of [[], [workflow.turns[1]], [workflow.turns[0], workflow.turns[0]], [...workflow.turns].reverse()]) {
      expect(decodePaperAnalysisWorkflow({ ...workflow, turns })).toBeUndefined();
    }
    expect(decodePaperAnalysisWorkflow({ ...singleStage, activeClientRequestId: 123 })).toBeUndefined();
  });

  it("decodes raw paper-reading debug records without assuming their provider shape", () => {
    const request = { messages: [{ role: "system", content: "prompt" }] };
    const response = { events: [{ delta: "answer" }] };
    expect(decodePaperAiReadingTurnDebugData({ turnId: "turn", rawRequest: request, rawResponse: response }))
      .toEqual({ turnId: "turn", rawRequest: request, rawResponse: response });
    expect(decodePaperAiReadingTurnDebugData({ turnId: "turn", rawRequest: request }))
      .toBeUndefined();
  });

  it("decodes per-turn model and real context-window snapshots", () => {
    const session = {
      id: "session", paperId: "paper", contextSnapshotId: "paper-context-hash",
      attachmentIds: ["attachment"], parseRunIds: ["parse"],
      providerId: "openai", modelId: "gpt-5.5", reasoningEffort: "high",
      bootstrapKind: "fresh_paper",
      conceptRetrievalDefault: false,
      title: "Question", status: "active", createdAt: "now", updatedAt: "now",
      turns: [{
        id: "turn", sessionId: "session", ordinal: 0, userMessage: "Question",
        assistantMessage: "Answer", providerId: "openai", modelId: "gpt-5.5",
        messages: [
          { id: "user-1", role: "user", content: "Question" },
          { id: "assistant-1", role: "assistant", content: "Answer" },
        ],
        reasoningEffort: "high", contextWindowTokens: 258_000,
        contextWindowSource: "provider_runtime", status: "succeeded",
        conceptRetrievalEnabled: false,
        inputTokens: 79_000, outputTokens: 2_000, createdAt: "now", finishedAt: "later",
      }],
    };
    const decoded = decodePaperAiReadingSessionDetail(session);
    expect(decoded?.contextSnapshotId).toBe("paper-context-hash");
    expect(decoded?.reasoningEffort).toBe("high");
    expect(decoded?.turns[0]).toMatchObject({
      reasoningEffort: "high",
      contextWindowTokens: 258_000,
      contextWindowSource: "provider_runtime",
    });
    expect(decodePaperAiReadingSessionDetail({
      ...session,
      turns: [{
        ...session.turns[0],
        assistantMessage: "",
        status: "canceled",
        errorCode: "canceled",
        errorMessage: "AI 运行已取消",
      }],
    })?.turns[0]?.status).toBe("canceled");
    expect(decodePaperAiReadingSessionDetail({
      ...session,
      turns: [{ ...session.turns[0], contextWindowSource: "invented" }],
    })).toBeUndefined();
    expect(decodePaperAiReadingSessionDetail({
      ...session,
      bootstrapKind: "structured_history",
    })).toBeUndefined();

    expect(decodePaperAiReadingSteerResult({
      status: "accepted",
      message: { id: "user-steer", role: "user", content: "Focus on the method" },
    })).toEqual({
      status: "accepted",
      message: { id: "user-steer", role: "user", content: "Focus on the method" },
    });
    expect(decodePaperAiReadingSteerResult({ status: "accepted" })).toBeUndefined();
    expect(decodePaperAiReadingCancelResult({
      accepted: true,
      resubmitMessages: [{ id: "user-steer", role: "user", content: "Focus on the method" }],
    })).toEqual({
      accepted: true,
      resubmitMessages: [{ id: "user-steer", role: "user", content: "Focus on the method" }],
    });

    expect(decodeLlmProviderModels({
      providerId: "qwen",
      models: [{
        id: "qwen-vl", displayName: "Qwen VL", description: "", isDefault: true,
        supportedReasoningEfforts: [], contextWindowTokens: 262_144,
        contextWindowSource: "models_dev",
      }],
    })?.models[0]?.contextWindowTokens).toBe(262_144);
  });

  it("retains catalog provenance and rejects malformed freshness metadata", () => {
    const model = { id: "fixture", displayName: "Fixture", description: "", isDefault: true, supportedReasoningEfforts: [] };
    const decode = (catalog: unknown) => decodeLlmProviderModels({ providerId: "openai", models: [model], catalog });
    const catalog = { source: "cache", stale: true, fetchedAt: 1234 };
    expect(decode(catalog)?.catalog).toEqual(catalog);
    expect(decode({ ...catalog, source: "unknown" })).toBeUndefined();
    expect(decode({ ...catalog, stale: "false" })).toBeUndefined();
    expect(decode({ ...catalog, fetchedAt: -1 })).toBeUndefined();
  });

  it("rejects malformed provider data instead of trusting TypeScript shapes", () => {
    expect(decodeAiProviderOverview({ settings: { providerId: "deepseek" }, capabilities: {} }))
      .toBeUndefined();
    expect(decodeAiCredentialSummary({ providerId: "unknown", configured: true }))
      .toBeUndefined();
  });

  it("preserves the native Fast capability and rejects malformed capability values", () => {
    const model = { id: "gpt-5.6-terra", displayName: "GPT-5.6-Terra", description: "", isDefault: true, supportedReasoningEfforts: ["medium"] };
    const decode = (supportsFastMode: unknown) => decodeLlmProviderModels({ providerId: "openai", models: [{ ...model, supportsFastMode }] });
    expect(decode(true)?.models[0]?.supportsFastMode).toBe(true);
    expect(decode(false)?.models[0]?.supportsFastMode).toBe(false);
    expect(decode(undefined)?.models[0]?.supportsFastMode).toBeUndefined();
    expect(decode("true")).toBeUndefined();
    expect(decode(null)).toBeUndefined();
  });

  it("rejects an API key field at the credential summary boundary", () => {
    const decoded = decodeAiCredentialSummary({
      providerId: "deepseek",
      configured: true,
      maskedValue: "••••-cret",
      apiKey: "must-not-cross-this-boundary",
    });
    expect(decoded).toBeUndefined();
  });

  it("decodes provider-level model configuration without exposing credentials", () => {
    const value = {
      providers: [{
        providerId: "qwen", displayName: "Qwen", authMode: "api-key", protocol: "openai-compatible",
        modelKinds: ["text", "embedding", "vision"],
        defaultBaseUrls: ["https://example.aliyuncs.com/compatible-mode/v1"],
        baseUrl: "https://example.aliyuncs.com/compatible-mode/v1",
        usesDefaultBaseUrl: true, enabled: false, timeoutMs: 60_000,
        credentialConfigured: true, credentialMask: "••••1234",
      }],
      defaultModels: [{
        routeKind: "default", routeId: "embedding", label: "Embedding 模型",
        modelKind: "embedding", selection: { providerId: "qwen", modelId: "text-embedding-v4" },
      }],
      useCases: [],
    };
    expect(decodeLlmConfiguration(value)?.providers[0]?.providerId).toBe("qwen");
    expect(decodeLlmConfiguration({ ...value, providers: [{ ...value.providers[0], apiKey: "secret" }] }))
      .toBeUndefined();
  });

  it("rejects unknown run and stage statuses", () => {
    const base = {
      id: "run", taskKind: "provider_diagnostic", status: "queued", providerId: "fixture-generation",
      modelId: "fixture-v1", pipelineVersion: "v1", promptVersion: "v1", outputSchemaVersion: 1,
      userGoal: "", scopeHash: "a".repeat(64), inputItemCount: 0, maxInputTokens: 128,
      maxOutputTokens: 64, timeoutMs: 5000, cancelRequested: false, createdAt: "now", updatedAt: "now",
      scopeItems: [], stages: [],
    };
    expect(decodeAiRun(base)?.status).toBe("queued");
    expect(decodeAiRun(base)).not.toHaveProperty("sharedConversationId");
    expect(decodeAiRun({
      ...base,
      sharedConversationId: "conversation",
      sharedRequestId: "request",
      sharedTurnId: "turn",
      sharedTurnOutcome: "completed",
      sharedCandidateId: "candidate",
      sharedTerminalStatus: "needs_review",
    })).toMatchObject({
      sharedConversationId: "conversation",
      sharedRequestId: "request",
      sharedTurnId: "turn",
      sharedTurnOutcome: "completed",
      sharedCandidateId: "candidate",
      sharedTerminalStatus: "needs_review",
    });
    expect(decodeAiRun({ ...base, sharedTurnOutcome: "invented" })).toBeUndefined();
    expect(decodeAiRun({ ...base, status: "secret_success" })).toBeUndefined();
    expect(decodeAiRun({ ...base, inputTokens: "12" })).toBeUndefined();
    expect(decodeAiRun({ ...base, inputTokens: 12 })?.inputTokens).toBe(12);
    const retry = { candidateId: "candidate", baseRevision: 2, targetFields: ["limitation"] };
    expect(decodeAiRun({ ...base, taskKind: "paper_overview", overviewRetry: retry })
      ?.overviewRetry?.targetFields).toEqual(["limitation"]);
    expect(decodeAiRun({ ...base, overviewRetry: { ...retry, targetFields: ["custom_evidence"] } })
      ?.overviewRetry?.targetFields).toEqual(["custom_evidence"]);
    expect(decodeAiRun({ ...base, overviewRetry: { ...retry, targetFields: ["invalid field"] } }))
      .toBeUndefined();
    const profile = {
      paperId: "paper-1",
      selectionRevision: 2,
      profileRefs: [{ profileId: "philosophical_conceptual", profileVersion: 1 }],
      fieldKinds: ["research_question", "argument", "objection", "response"],
    };
    expect(decodeAiRun({ ...base, taskKind: "paper_overview", overviewProfile: profile })
      ?.overviewProfile?.fieldKinds).toContain("argument");
    expect(decodeAiRun({ ...base, overviewProfile: { ...profile, fieldKinds: ["unknown"] } }))
      .toBeUndefined();
    expect(decodeAiRun({
      ...base,
      taskKind: "paper_suggestions",
      suggestionCandidateKind: "concept_suggestion_v1",
    })?.suggestionCandidateKind).toBe("concept_suggestion_v1");
    expect(decodeAiRun({ ...base, taskKind: "paper_suggestions" })).toBeUndefined();
    expect(decodeAiRun({
      ...base,
      suggestionCandidateKind: "concept_suggestion_v1",
    })).toBeUndefined();
    expect(decodeAiRun({
      ...base,
      taskKind: "paper_suggestions",
      suggestionCandidateKind: "invented_suggestion_v1",
    })).toBeUndefined();
    const matrixContext = {
      projectId: "project", matrixId: "matrix", matrixRevision: 1,
      viewKind: "taxonomy", title: "Taxonomy", scopeNote: "Selected cells only",
      inputs: [
        { objectKind: "matrix_cell", objectId: "cell-a", objectRevision: 1, selectionReason: "" },
        { objectKind: "matrix_cell", objectId: "cell-b", objectRevision: 2, selectionReason: "" },
      ],
      evidenceAnchorIds: ["anchor-a", "anchor-b"],
    };
    expect(decodeAiRun({
      ...base, taskKind: "matrix_synthesis", matrixSynthesisContext: matrixContext,
    })?.matrixSynthesisContext?.inputs).toHaveLength(2);
    expect(decodeAiRun({ ...base, taskKind: "matrix_synthesis" })).toBeUndefined();
    expect(decodeAiRun({
      ...base, matrixSynthesisContext: matrixContext,
    })).toBeUndefined();
    expect(decodeAiRun({
      ...base, taskKind: "matrix_synthesis",
      matrixSynthesisContext: { ...matrixContext, evidenceAnchorIds: ["anchor-a", "anchor-a"] },
    })).toBeUndefined();
  });

  it("requires the exact outgoing evidence preview for selection tasks", () => {
    const preview = {
      taskKind: "selection_explanation", providerId: "fixture-generation", modelId: "fixture-v1",
      isCloud: false, scopeHash: "a".repeat(64), inputItemCount: 1, scopeItems: [],
      requiresCloudConsent: false,
      evidencePreview: [{
        citationKey: "ev-quote", anchorId: "anchor", paperTitle: "Paper",
        attachmentId: "attachment", contentSha256: "b".repeat(64), pageIndex: 2,
        quoteExact: "Exact quote", contextText: "Exact quote", relationKind: "direct_quote",
      }],
    };
    expect(decodeAiTaskPreview(preview)?.evidencePreview[0]?.quoteExact).toBe("Exact quote");
    expect(decodeAiTaskPreview({ ...preview, taskKind: "paper_overview", overviewProfile: {
      paperId: "paper", selectionRevision: 1,
      profileRefs: [{ profileId: "generic_overview", profileVersion: 1 }],
      fieldKinds: ["research_question"],
    } })?.overviewProfile?.selectionRevision).toBe(1);
    expect(decodeAiTaskPreview({ ...preview, evidencePreview: undefined })).toBeUndefined();
    expect(decodeAiTaskPreview({
      ...preview,
      taskKind: "paper_suggestions",
      suggestionCandidateKind: "attention_suggestion_v1",
    })?.suggestionCandidateKind).toBe("attention_suggestion_v1");
    expect(decodeAiTaskPreview({ ...preview, taskKind: "paper_suggestions" })).toBeUndefined();
    expect(decodeAiTaskPreview({
      ...preview,
      suggestionCandidateKind: "attention_suggestion_v1",
    })).toBeUndefined();
    const matrixContext = {
      projectId: "project", matrixId: "matrix", matrixRevision: 1,
      viewKind: "historical_context", title: "History", scopeNote: "Selected cells only",
      inputs: [
        { objectKind: "matrix_cell", objectId: "cell-a", objectRevision: 1, selectionReason: "" },
        { objectKind: "matrix_cell", objectId: "cell-b", objectRevision: 1, selectionReason: "" },
      ],
      evidenceAnchorIds: ["anchor-a", "anchor-b"],
    };
    expect(decodeAiTaskPreview({
      ...preview, taskKind: "matrix_synthesis", matrixSynthesisContext: matrixContext,
    })?.matrixSynthesisContext?.viewKind).toBe("historical_context");
    expect(decodeAiTaskPreview({ ...preview, matrixSynthesisContext: matrixContext }))
      .toBeUndefined();
  });
});
