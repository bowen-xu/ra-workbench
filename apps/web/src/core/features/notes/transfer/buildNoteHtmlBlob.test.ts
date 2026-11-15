import { expect, it } from "vitest";
import { buildNoteHtmlBlob } from "./buildNoteHtmlBlob";
import type { NoteExportInput } from "./noteExportInput";
it("keeps formula HTML and original image bytes while assembling large fonts off the UI thread", async () => {
    const font = new Uint8Array(40001);
    for (let index = 0; index < font.length; index++)
        font[index] = index % 256;
    const image = new TextEncoder().encode('<svg xmlns="http://www.w3.org/2000/svg"/>');
    const input: NoteExportInput = { template: '<style>@font-face{src:url(FONT)}</style><span class="katex"><sub>1</sub></span><img src="IMAGE">', resources: [{ token: "FONT", kind: "font", name: "math.ttf" }, { token: "IMAGE", kind: "image", assetId: "asset" }] };
    const blob = await buildNoteHtmlBlob(input, async (resource) => resource.kind === "font" ? font : image);
    const html = await blob.text();
    expect(html).toContain('<span class="katex"><sub>1</sub></span>');
    const encoded = html.match(/data:font\/ttf;base64,([A-Za-z0-9+/=]+)/u)![1]!;
    expect(Uint8Array.from(atob(encoded), character => character.charCodeAt(0))).toEqual(font);
    expect(html).toContain('data:image/svg+xml;base64,');
    expect(blob.type).toBe("text/html;charset=utf-8");
});
it("checks cancellation between binary chunks and never returns a partial document", async () => {
    const input: NoteExportInput = { template: "FONT", resources: [{ token: "FONT", kind: "font", name: "math.ttf" }] };
    let checks = 0;
    await expect(buildNoteHtmlBlob(input, async () => new Uint8Array(50000), () => { if (++checks === 4)
        throw new DOMException("stopped", "AbortError"); })).rejects.toMatchObject({ name: "AbortError" });
});
