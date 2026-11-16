import portrait from "../../../../desktop/agent-resources/skills/generate-paper-portrait/SKILL.md?raw";
import editing from "../../../../desktop/agent-resources/skills/edit-structured-resources/SKILL.md?raw";
import portraitReference from "../../../../desktop/agent-resources/skills/edit-structured-resources/references/portrait.md?raw";
import readingReference from "../../../../desktop/agent-resources/skills/edit-structured-resources/references/reading.md?raw";
import templatesReference from "../../../../desktop/agent-resources/skills/edit-structured-resources/references/templates.md?raw";
export const SAMPLE_PDF_URL = "/preview/sample.pdf";

export async function readSamplePdf(): Promise<Uint8Array> {
  const response = await fetch(SAMPLE_PDF_URL);
  if (!response.ok) throw new Error("预览 PDF 静态样本无法加载");
  return new Uint8Array(await response.arrayBuffer());
}

export async function hashPreviewBytes(content: string | Uint8Array): Promise<string> {
  const bytes = await crypto.subtle.digest("SHA-256", typeof content === "string" ? new TextEncoder().encode(content) : Uint8Array.from(content));
  return [...new Uint8Array(bytes)].map(byte => byte.toString(16).padStart(2, "0")).join("");
}

export const previewSkillSources = {portrait, editing, portraitReference, readingReference, templatesReference};
