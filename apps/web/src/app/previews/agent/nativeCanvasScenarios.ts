import type { NativeAgentPreviewSample } from "../../../core/runtime/nativeAgentPreview";

export const nativeCanvasScenarioOptions = [
  ["text", "流式正文"], ["multiple", "多项与阶段"], ["tool", "工具续轮"], ["interleavedTools", "流内连续工具"], ["lateFailure", "完成工具后断流"],
  ["sparse", "稀疏完成项"], ["interaction", "追加输入与队列"], ["toolCorrection", "工具参数纠错"],
  ["unknownTool", "未知工具纠错"], ["namespacedTool", "工具命名空间纠错"], ["reasoningSummary", "公开思考摘要"],
  ["reasoningSparse", "稀疏思考摘要"], ["reasoningRetry", "思考摘要断流重试"], ["compactSuccess", "上下文压缩"], ["compactInvalid", "非法压缩候选"], ["compactLegacy", "旧式远程压缩"],
  ["fileDelivery", "稿件文件交付"], ["fileDeliveryTwice", "稿件连续交付"],
  ["httpAuth", "模型认证失败 · HTTP 401"], ["httpRateLimit", "模型限流后重试 · HTTP 429"],
  ["httpUnavailable", "模型服务不可用 · HTTP 503"],
] as const;
export type NativeCanvasScenario = typeof nativeCanvasScenarioOptions[number][0];
export function nativeCanvasScenarioSupported(scenario: NativeCanvasScenario, supported: readonly string[]): boolean {
  if (scenario === "interleavedTools") return supported.includes("in-stream-tool-receipts");
  if (scenario === "unknownTool") return supported.includes("unknown-tool-feedback");
  if (scenario === "namespacedTool") return supported.includes("tool-namespace-facts");
  if (scenario === "reasoningSummary" || scenario === "reasoningSparse" || scenario === "reasoningRetry") return supported.includes("public-reasoning-summary");
  if (scenario === "compactSuccess" || scenario === "compactInvalid") return supported.includes("manual-compaction");
  if (scenario === "compactLegacy") return supported.includes("manual-compaction") && supported.includes("legacy-compaction-config-fixture");
  if (scenario === "fileDelivery") return supported.includes("native-file-applications");
  if (scenario === "fileDeliveryTwice") return supported.includes("native-file-applications") && supported.includes("successor-workspace-delivery");
  return true;
}
function event(value: unknown): string { return `data: ${JSON.stringify(value)}\n\n`; }
function message(id: string, text: string, phase: string) { return { type: "message", id, role: "assistant", phase, content: [{ type: "output_text", text, annotations: [] }] }; }
function start(id: string): string { return event({ type: "response.created", response: { id } }); }
function finish(id: string): string { return event({ type: "response.completed", response: { id, status: "completed", usage: { input_tokens: 40, output_tokens: 16 } } }); }
function done(item: unknown): string { return event({ type: "response.output_item.done", item }); }
/** Provider i64 JSON literals must reach Rust intact, before JavaScript Number parsing. */
function summaryProgress(type: string, itemId: string, index: string, text?: string): string {
  return `data: {"type":${JSON.stringify(type)},"item_id":${JSON.stringify(itemId)},"summary_index":${index}${text === undefined ? "" : `,${type.endsWith(".done") ? '"text"' : '"delta"'}:${JSON.stringify(text)}`}}\n\n`;
}
function reasoningItem(id: string, parts: readonly string[]) {
  return { type: "reasoning", id, summary: parts.map(text => ({ type: "summary_text", text })), encrypted_content: "synthetic-opaque-never-render" };
}
function textSample(id: string, text: string): NativeAgentPreviewSample {
  const item = message(`${id}-message`, text, "final_answer");
  return { sse: start(id) + event({ type: "response.output_item.added", item: { ...item, content: [] } })
    + event({ type: "response.output_text.delta", item_id: item.id, delta: text }) + done(item) + finish(id), chunkBytes: 64, delayMs: 20 };
}

/** Synthetic model request only; the real note reader checks the selected scope. */
export function nativeCanvasNoteReadSamples(noteId: string): readonly NativeAgentPreviewSample[] {
  const id = `preview-note-read-${crypto.randomUUID()}`;
  return [{sse:start(id) + done({type:"function_call",id:`${id}-item`,call_id:`${id}-call`,name:"research_read_file",
    arguments:JSON.stringify({path:`source://notes/${noteId}`})}) + finish(id),chunkBytes:64,delayMs:20},
  textSample(`${id}-final`, "这是隔离原生预览的回复。笔记读取由实际 Agent 工具执行。")];
}

/** Only provider wire is synthetic; the MCP request uses the native client. */
export function nativeCanvasMcpSamples(): readonly NativeAgentPreviewSample[] {
  const id = `preview-mcp-${crypto.randomUUID()}`;
  return [{sse:start(id) + done({type:"function_call",id:`${id}-item`,call_id:`${id}-call`,
    namespace:"mcp__canvas_fixture",name:"find_doc",arguments:"{}"}) + finish(id),chunkBytes:64,delayMs:20},
  textSample(`${id}-final`, "合成 MCP 工具调用已结束，请查看工具结果。")];
}

/** Raw provider bytes only. Rust owns decoding, scheduling, persistence and retry. */
export function nativeCanvasSamples(scenario: NativeCanvasScenario, context: "writing" | "independent" = "writing"): readonly NativeAgentPreviewSample[] {
  const scope = crypto.randomUUID();
  const final = textSample(`preview-final-${scope}`, "这是隔离原生预览的回复。正文经过实际 Rust 解码、调度、Raft 接纳和 SQLite 物化。");
  if (scenario === "httpAuth" || scenario === "httpRateLimit" || scenario === "httpUnavailable") {
    const status = scenario === "httpAuth" ? 401 : scenario === "httpRateLimit" ? 429 : 503;
    const failure: NativeAgentPreviewSample = { sse: "", httpStatus: status, chunkBytes: 64, delayMs: 0 };
    return scenario === "httpAuth" ? [failure] : scenario === "httpRateLimit" ? [failure, final] : Array.from({length: 6}, () => ({...failure}));
  }
  if (scenario === "interleavedTools") {
    const id = `preview-interleaved-${scope}`;
    const tool = (index: number) => ({ type: "function_call", id: `${id}-item-${index}`,
      call_id: `${id}-call-${index}`, name: "research_list_sources", arguments: "{}" });
    return [{ sse: start(id) + done(tool(1))
      + done(message(`${id}-progress`, "第一个工具完成后，继续处理同一响应中的下一项。", "commentary"))
      + done(tool(2)) + ": preview pacing\n\n".repeat(128) + finish(id), chunkBytes: 64, delayMs: 100 }, final];
  }
  if (scenario === "fileDelivery") {
    const id = `preview-file-delivery-${scope}`;
    return [{sse: start(id) + done({type: "function_call", id: `${id}-item`, call_id: `${id}-call`, name: "research_exec_command",
      arguments: JSON.stringify({command: "printf '%s\\n' '\\documentclass{article}' '\\keywords{research, evidence}' '' > sn-article.tex", yield_ms: 5000})})
      + finish(id), chunkBytes: 64, delayMs: 200},
    {sse: start(`${id}-deliver`) + done({type: "function_call", id: `${id}-deliver-item`, call_id: `${id}-deliver-call`,
      name: "research_deliver_files", arguments: "{}"}) + finish(`${id}-deliver`), chunkBytes: 64, delayMs: 20}, final];
  }
  if (scenario === "fileDeliveryTwice") {
    const id = `preview-file-delivery-twice-${scope}`;
    const call = (suffix: string, name: string, args: unknown, delayMs: number): NativeAgentPreviewSample => {
      const response = `${id}-${suffix}`;
      return { sse: start(response) + done({ type: "function_call", id: `${response}-item`, call_id: `${response}-call`,
        name, arguments: JSON.stringify(args) }) + finish(response), chunkBytes: 64, delayMs };
    };
    const write = (keywords: string) => ({ command: `printf '%s\\n' '\\documentclass{article}' '\\keywords{research, ${keywords}}' '' > sn-article.tex`, yield_ms: 5000 });
    return [call("first-write", "research_exec_command", write("evidence"), 200),
      call("first-delivery", "research_deliver_files", {}, 20),
      call("second-write", "research_exec_command", write("verification"), 200),
      call("second-delivery", "research_deliver_files", {}, 20), final];
  }
  if (scenario === "compactSuccess") return [{ sse: start(`preview-compact-${scope}`)
    + done({ type: "compaction", id: `preview-compact-item-${scope}`, encrypted_content: "synthetic-preview-compaction" })
    + finish(`preview-compact-${scope}`), chunkBytes: 64, delayMs: 20 }];
  if (scenario === "compactLegacy") return [textSample(`preview-legacy-seed-${scope}`, "这段合成对话用于准备待压缩的历史。"),
    {sse: JSON.stringify({output: [{type: "compaction", encrypted_content: "synthetic-preview-legacy-compaction"}]}), chunkBytes: 64, delayMs: 20}];
  if (scenario === "compactInvalid") return [{ sse: start(`preview-invalid-compact-${scope}`)
    + done({ type: "compaction", id: `preview-invalid-compact-item-${scope}`, encrypted_content: 42 })
    + finish(`preview-invalid-compact-${scope}`), chunkBytes: 64, delayMs: 20 }];
  if (scenario === "text") return [final];
  if (scenario === "interaction") return [textSample(`preview-first-${scope}`, "先处理第一条消息。追加消息由实际队列与同轮输入机制接纳。"), final,
    textSample(`preview-followup-${scope}`, "已完成后续输入的处理。")];
  if (scenario === "multiple") return [{ sse: start(`preview-multiple-${scope}`)
    + done({ type: "reasoning", id: `preview-reasoning-${scope}`, encrypted_content: "synthetic-opaque", summary: [] })
    + done(message(`preview-commentary-${scope}`, "先检查当前稿件，再给出结论。", "commentary"))
    + done(message(`preview-answer-${scope}`, "检查完成。这是同一响应中的独立最终项。", "final_answer")) + finish(`preview-multiple-${scope}`), chunkBytes: 64, delayMs: 20 }];
  if (scenario === "sparse") return [{ sse: done(message(`preview-sparse-${scope}`, "这段正文只存在于稀疏完成项中。", "final_answer")) + finish(`preview-sparse-${scope}`), chunkBytes: 64, delayMs: 20 }];
  if (scenario === "reasoningSparse") return [{ sse: start(`preview-summary-sparse-${scope}`)
    + done(reasoningItem(`preview-summary-empty-${scope}`, []))
    + done(reasoningItem(`preview-summary-sparse-item-${scope}`, ["仅完成项中的公开摘要。", "第二部分保留 🦀 UTF-8 边界。 "]))
    + done(message(`preview-summary-sparse-answer-${scope}`, "稀疏公开摘要完成；这段是独立最终回答。", "final_answer")) + finish(`preview-summary-sparse-${scope}`), chunkBytes: 64, delayMs: 20 }];
  if (scenario === "reasoningRetry") {
    const id = `preview-summary-failed-${scope}`;
    return [{ sse: start(id) + event({ type: "response.output_item.added", item: reasoningItem(`${id}-item`, []) })
      + summaryProgress("response.reasoning_summary_text.delta", `${id}-item`, "-9223372036854775808", "断流前的公开摘要仍未完成。"), chunkBytes: 64, delayMs: 20 },
    { sse: start(`preview-summary-retry-${scope}`) + done(reasoningItem(`preview-summary-retry-item-${scope}`, ["重试后独立完成的公开摘要。"]))
      + done(message(`preview-summary-retry-answer-${scope}`, "重试成功，摘要没有变成最终回答。", "final_answer")) + finish(`preview-summary-retry-${scope}`), chunkBytes: 64, delayMs: 20 }];
  }
  if (scenario === "reasoningSummary") {
    const id = `preview-summary-${scope}`, itemId = `${id}-item`;
    return [{ sse: start(id) + event({ type: "response.output_item.added", item: reasoningItem(itemId, []) })
      + summaryProgress("response.reasoning_summary_part.added", itemId, "9223372036854775807")
      + summaryProgress("response.reasoning_summary_text.delta", itemId, "9223372036854775807", "公开部分 A1。")
      + summaryProgress("response.reasoning_summary_text.delta", itemId, "-9223372036854775808", "公开部分 B。")
      + summaryProgress("response.reasoning_summary_text.delta", itemId, "9223372036854775807", "追加 A2。")
      + summaryProgress("response.reasoning_summary_text.done", itemId, "9223372036854775807", "公开部分 A1。追加 A2。")
      + done(reasoningItem(itemId, ["公开摘要最终校正 A。", "公开摘要第二部分 B 🦀。 "]))
      + done(reasoningItem(`${id}-second`, ["同轮第二个独立公开摘要。"]))
      + done(message(`${id}-answer`, "公开摘要已完成；最终回答仅显示这里的正文。", "final_answer")) + finish(id), chunkBytes: 64, delayMs: 20 }];
  }
  if (scenario === "toolCorrection") return ["{", "{}"].map((argumentsJson, index): NativeAgentPreviewSample => {
    const id = `preview-correction-${scope}-${index}`;
    return { sse: start(id) + done({ type: "function_call", id: `${id}-item`, call_id: `${id}-call`, name: "research_list_sources", arguments: argumentsJson }) + finish(id), chunkBytes: 64, delayMs: 20 };
  }).concat(final);
  if (scenario === "unknownTool" || scenario === "namespacedTool") return [0, 1].map((index): NativeAgentPreviewSample => {
    const id = `preview-unsupported-${scope}-${index}`;
    const item = { type: "function_call", id: `${id}-item`, call_id: `${id}-call`, arguments: "{}",
      name: index === 0 && scenario === "unknownTool" ? "preview_unknown_tool" : "research_list_sources",
      ...(index === 0 && scenario === "namespacedTool" ? { namespace: "foreign::" } : {}) };
    return { sse: start(id) + done(item) + finish(id), chunkBytes: 64, delayMs: 20 };
  }).concat(final);
  const first = start(`preview-tool-${scope}`) + done(message(`preview-progress-${scope}`, context === "independent" ? "先查看已授权的来源。" : "先读取稿件的文件列表。", "commentary"))
    + done({ type: "function_call", id: `preview-function-${scope}`, call_id: `preview-call-${scope}`, name: "research_list_sources", arguments: "{}" });
  if (scenario === "lateFailure") return [{ sse: first, chunkBytes: 64, delayMs: 20 }, final];
  return [{ sse: first + finish(`preview-tool-${scope}`), chunkBytes: 64, delayMs: 20 }, final];
}
