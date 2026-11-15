/** Derive a short initial title from the first ordinary user message. */
export function summarizeConversationTitle(text: string): string {
  const normalized = text.replace(/\s+/gu, " ").trim().replace(/^[，。！？、：；,.!?;:]+|[，。！？、：；,.!?;:]+$/gu, "");
  if (!normalized) return "新对话";
  const concise = normalized.replace(/^(请问|请帮我|可以帮我|帮我|请|能否)\s*/u, "").trim() || normalized;
  return Array.from(concise).slice(0, 28).join("") + (Array.from(concise).length > 28 ? "…" : "");
}
