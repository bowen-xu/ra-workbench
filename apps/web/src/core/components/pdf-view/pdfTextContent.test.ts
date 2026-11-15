import { afterEach, describe, expect, it } from "vitest";
import { getDocument } from "pdfjs-dist/legacy/build/pdf.mjs";

import {
  pdfSourceCharacters,
  pdfWorkerCharacterCapability,
  readPdfTextContent,
  type PageTextContent,
} from "./pdfTextContent";

function minimalPdf(text: string): Uint8Array {
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    `<< /Length ${`BT /F1 24 Tf 40 300 Td (${text}) Tj ET`.length} >>\nstream\nBT /F1 24 Tf 40 300 Td (${text}) Tj ET\nendstream`,
  ];
  let body = "%PDF-1.4\n";
  const offsets = [0];
  objects.forEach((object, index) => {
    offsets.push(new TextEncoder().encode(body).length);
    body += `${index + 1} 0 obj\n${object}\nendobj\n`;
  });
  const xrefOffset = new TextEncoder().encode(body).length;
  body += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  body += offsets.slice(1).map((offset) => `${String(offset).padStart(10, "0")} 00000 n \n`).join("");
  body += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xrefOffset}\n%%EOF\n`;
  return new TextEncoder().encode(body);
}

let loadingTask: ReturnType<typeof getDocument> | undefined;

afterEach(async () => {
  await loadingTask?.destroy();
  loadingTask = undefined;
});

describe("PDF worker character geometry", () => {
  it("returns per-glyph PDF coordinates from the patched worker without consulting a text-layer DOM", async () => {
    loadingTask = getDocument({ data: minimalPdf("ALIGN") });
    const document = await loadingTask.promise;
    const page = await document.getPage(1);
    const content = await readPdfTextContent(page);
    const chars = pdfSourceCharacters(content, page.getViewport({ scale: 1 }));

    expect(chars.map((char) => char.value).join("")).toBe("ALIGN");
    expect(chars).toHaveLength(5);
    expect(chars.every((char) => char.rect.width > 0 && char.rect.height > 0)).toBe(true);
    expect(chars[0]?.rect.x).toBeCloseTo(40 / 300, 3);
    expect(chars[0]?.rect.y).toBeGreaterThan(0);
    expect(chars[0]?.rect.y).toBeLessThan(1);

    const whitespaceItem = {
      ...content.items[0],
      str: " ",
      chars: [],
    } as unknown as PageTextContent["items"][number];
    expect(pdfWorkerCharacterCapability({
      ...content,
      items: [...content.items, whitespaceItem],
    })).toBe("available");
  });
});
