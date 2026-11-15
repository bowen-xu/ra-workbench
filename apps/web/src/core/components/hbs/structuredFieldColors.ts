/** Stable field identities keep their category color when a template is reordered or renamed. */
export const READING_FIELD_PALETTE = ["#4d8d5b", "#b98528", "#b76184", "#437cb1", "#956e44", "#7964ad", "#397f83", "#ad6652", "#687e36"] as const;

export function readingFieldColor(fieldId: string, configured?: string): string {
  if (configured !== undefined && /^#[0-9a-fA-F]{6}$/.test(configured)) return configured;
  const colors: Readonly<Record<string, string>> = {
    question: "#4d8d5b", method: "#b98528", result: "#b76184", concepts: "#7964ad", formal_model: "#437cb1", assumptions: "#397f83", recognition_learning: "#687e36", evaluation: "#b98528", findings: "#b76184", limitations: "#956e44", open_questions: "#ad6652",
    research_question: "#4d8d5b", approach: "#b98528", key_result: "#b76184", conclusion: "#437cb1",
    limitation: "#956e44", open_question: "#7964ad", argument: "#397f83", objection: "#ad6652", response: "#687e36",
  };
  if (colors[fieldId] !== undefined) return colors[fieldId];
  let hash = 0;
  for (const character of fieldId) hash = (hash * 31 + character.charCodeAt(0)) >>> 0;
  return READING_FIELD_PALETTE[hash % READING_FIELD_PALETTE.length]!;
}
