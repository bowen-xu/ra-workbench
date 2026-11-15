import type { WordExplanation, WordExplanationGateway, WordFollowUpReply } from "@research-assistant/core/dictionary";
import { resolveAppGateway } from "../runtime/appRuntime";
import { invoke, isDesktopRuntime } from "../runtime/platformCore";

function metadata(value: unknown): Pick<WordFollowUpReply, "providerId" | "modelId" | "modelRevision"> {
  if (typeof value !== "object" || value === null || !("providerId" in value) || !("modelId" in value)
    || typeof value.providerId !== "string" || typeof value.modelId !== "string"
    || ("modelRevision" in value && value.modelRevision !== undefined && typeof value.modelRevision !== "string")) throw new Error("AI 模型信息无效。");
  return { providerId: value.providerId, modelId: value.modelId,
    ...("modelRevision" in value && typeof value.modelRevision === "string" ? { modelRevision: value.modelRevision } : {}) };
}
function boundedText(value: unknown, limit: number): value is string {
  return typeof value === "string" && value.trim().length > 0 && [...value].length <= limit;
}
function decodeExplanation(value: unknown): WordExplanation {
  if (typeof value !== "object" || value === null || !("translation" in value) || !("explanation" in value)
    || !boundedText(value.translation, 120) || !boundedText(value.explanation, 1000)) throw new Error("AI 词义解释响应无效。");
  return { translation: value.translation, explanation: value.explanation, ...metadata(value) };
}
function decodeReply(value: unknown): WordFollowUpReply {
  if (typeof value !== "object" || value === null || !("text" in value) || !boundedText(value.text, 2000)) throw new Error("AI 追问响应无效。");
  return { text: value.text, ...metadata(value) };
}
async function requestOwned<T>(command: string, request: unknown, decode: (value: unknown) => T, signal?: AbortSignal): Promise<T> {
  signal?.throwIfAborted();
  const requestId = crypto.randomUUID();
  const cancel = (): void => { void invoke("cancel_word_explanation", { requestId }).catch(() => undefined); };
  signal?.addEventListener("abort", cancel, { once: true });
  try {
    const value = await invoke<unknown>(command, { requestId, request });
    signal?.throwIfAborted();
    return decode(value);
  } catch (error) {
    signal?.throwIfAborted();
    const message = typeof error === "object" && error !== null && "message" in error && typeof error.message === "string"
      ? error.message : "AI 词义解释失败，请检查阅读模型配置后重试。";
    throw new Error(message);
  } finally { signal?.removeEventListener("abort", cancel); }
}
const desktop: WordExplanationGateway<AbortSignal> = {
  explain: (request, signal) => requestOwned("explain_dictionary_word", request, decodeExplanation, signal),
  followUp: (request, signal) => requestOwned("follow_up_dictionary_word", request, decodeReply, signal),
};
async function unavailable(): Promise<never> { throw new Error("AI 语境释义需要桌面应用及已配置的阅读模型；离线词典仍可使用。"); }
const browser: WordExplanationGateway<AbortSignal> = { explain: unavailable, followUp: unavailable };

export function getWordExplanationGateway(): WordExplanationGateway<AbortSignal> {
  return resolveAppGateway("getWordExplanationGateway", isDesktopRuntime() ? desktop : browser);
}
