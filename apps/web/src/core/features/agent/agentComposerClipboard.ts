import { agentFileSourceLink, agentDraftFromInput, agentDraftInput, type AgentInputDraft } from "@research-assistant/core/agent";
import { notify } from "../../services/notifications";
import { copyText } from "./copyText";

const draftMime = "application/x-research-assistant-agent-draft+json";
const draftAttribute = "data-research-assistant-agent-draft";
const maxPayloadBytes = 2_097_152;
type ClipboardData = Pick<DataTransfer, "getData" | "setData">;

/** Clipboard spans are relative to the copied fragment, not the source editor. */
export function composerClipboardSelection(draft: AgentInputDraft, from: number, to: number) {
  for (const binding of draft.bindings) {
    if (from < binding.endUtf16 && to > binding.startUtf16
      || from === to && from > binding.startUtf16 && from < binding.endUtf16) {
      from = Math.min(from, binding.startUtf16); to = Math.max(to, binding.endUtf16);
    }
  }
  const selected: AgentInputDraft = {text: draft.text.slice(from, to), bindings: draft.bindings
    .filter(binding => binding.startUtf16 >= from && binding.endUtf16 <= to)
    .map(binding => ({...binding, startUtf16: binding.startUtf16 - from, endUtf16: binding.endUtf16 - from}))};
  return {from, to, draft: selected};
}

function withinBudget(value: string): boolean {
  return value.length <= maxPayloadBytes && new TextEncoder().encode(value).length <= maxPayloadBytes;
}
function escapeHtml(value: string): string {
  return value.replace(/[&<>"']/gu, character => ({"&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;"})[character]!);
}

/** Only user copy/cut events write data. No skill contents or authorization are copied. */
export function writeComposerClipboard(data: ClipboardData, draft: AgentInputDraft, richHTML?: string): boolean {
  try {
    const payload = JSON.stringify({version: 1, text: draft.text, input: agentDraftInput(draft)});
    if (!withinBudget(payload)) throw new Error("Clipboard reference budget exceeded");
    data.setData("text/plain", draft.text);
    // HTML is an OS clipboard format, including in WebKit. Custom MIME is an
    // additional carrier, since native clipboard bridges may drop custom types.
    data.setData("text/html", richHTML === undefined
      ? `<span ${draftAttribute}="${escapeHtml(payload)}" style="white-space: pre-wrap">${escapeHtml(draft.text)}</span>`
      : `<div ${draftAttribute}="${escapeHtml(payload)}">${richHTML}</div>`);
    try { data.setData(draftMime, payload); }
    catch { return true; /* The standard HTML carrier already preserves the payload. */ }
    return true;
  } catch {
    notify.error("无法完整复制引用信息，输入内容已保留。请重试或缩小选择范围。");
    return false;
  }
}

/** Message copy buttons use the same HTML carrier as native editor copy events. */
export async function copyComposerClipboard(draft: AgentInputDraft): Promise<boolean> {
  const data = new Map<string, string>();
  if (!writeComposerClipboard({getData: type => data.get(type) ?? "", setData: (type, value) => {data.set(type, value);}}, draft)) return false;
  return copyText(draft.text, {"text/html": data.get("text/html")!});
}

/** External HTML is never rendered or attached to the document. Invalid metadata
 * falls back to ordinary paste with feedback; it cannot confer resource access. */
export function readComposerClipboard(data: Pick<DataTransfer, "getData">): AgentInputDraft | null {
  let payload = data.getData(draftMime);
  if (!payload) {
    const html = data.getData("text/html");
    if (!html.includes(draftAttribute)) return null;
    if (html.length > maxPayloadBytes * 6) {
      notify.error("剪贴板引用信息过大，将按普通文字粘贴。"); return null;
    }
    // A detached template keeps scripts, images and embedded resources inert.
    const template = document.createElement("template"); template.innerHTML = html;
    const carriers = template.content.querySelectorAll(`[${draftAttribute}]`);
    if (carriers.length !== 1) {
      notify.error("剪贴板引用信息无效，将按普通文字粘贴。"); return null;
    }
    payload = carriers[0]!.getAttribute(draftAttribute) ?? "";
  }
  try {
    if (!withinBudget(payload)) throw new Error("Clipboard reference budget exceeded");
    const value: unknown = JSON.parse(payload);
    if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("Invalid clipboard draft");
    const record = value as Record<string, unknown>;
    if (Object.keys(record).some(key => !["version", "text", "input"].includes(key))
      || record.version !== 1 || typeof record.text !== "string" || record.text.includes("\r") || !Array.isArray(record.input)
      || record.text.replace(/\r\n?/gu, "\n") !== data.getData("text/plain").replace(/\r\n?/gu, "\n")) throw new Error("Clipboard text mismatch");
    const draft = agentDraftFromInput(record.text, record.input);
    if (!draft.bindings.length || draft.unboundReferences?.length
      || draft.bindings.some(binding => {
        const text = draft.text.slice(binding.startUtf16, binding.endUtf16);
        const input=binding.input;
        const path = agentFileSourceLink(input);
        return text !== `@${input.name}` && text !== input.name && text !== path;
      })) throw new Error("Invalid clipboard references");
    return draft;
  } catch {
    notify.error("剪贴板引用信息无效，将按普通文字粘贴。"); return null;
  }
}

/** Replace an exact selection, retaining bindings outside it even for identical text. */
export function pasteComposerDraft(draft: AgentInputDraft, fragment: AgentInputDraft, start: number, end: number) {
  const {from, to} = composerClipboardSelection(draft, start, end);
  const delta = fragment.text.length - (to - from);
  const bindings = draft.bindings.flatMap(binding => binding.endUtf16 <= from ? [binding]
    : binding.startUtf16 >= to ? [{...binding, startUtf16: binding.startUtf16 + delta, endUtf16: binding.endUtf16 + delta}] : []);
  const next: AgentInputDraft = {...draft, text: draft.text.slice(0, from) + fragment.text + draft.text.slice(to),
    bindings: [...bindings, ...fragment.bindings.map(binding => ({...binding,
      startUtf16: binding.startUtf16 + from, endUtf16: binding.endUtf16 + from}))].sort((a, b) => a.startUtf16 - b.startUtf16)};
  return {from, to, draft: next, caret: from + fragment.text.length};
}
