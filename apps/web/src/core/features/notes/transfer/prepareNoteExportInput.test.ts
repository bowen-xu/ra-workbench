// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { prepareNoteExportInput } from "./prepareNoteExportInput";
import { createNotePdfSnapshot } from "./notePdfSnapshot";
beforeEach(() => { document.head.innerHTML = '<style>:root{background:#f1f0ea}.katex .vlist{position:relative;vertical-align:baseline}</style>'; });
afterEach(() => { document.head.innerHTML = ""; document.body.innerHTML = ""; vi.restoreAllMocks(); });
function source() { const node = document.createElement("div"); node.innerHTML = `<p><span class="katex"><span class="vlist" style="top:-2.7em">公式</span></span><img class="ProseMirror-separator"></p><figure data-asset-id="${"a".repeat(64)}"><img src="blob:current-image"></figure>`; return node; }
it("preserves native formula layout while sending only resource IDs and small placeholders", async () => {
    const live = source();
    const before = live.outerHTML;
    const decode = vi.spyOn(HTMLImageElement.prototype, "decode");
    const canvas = vi.spyOn(HTMLCanvasElement.prototype, "getContext");
    const input = await prepareNoteExportInput(createNotePdfSnapshot(live, "标题"), "标题", new AbortController().signal);
    expect(live.outerHTML).toBe(before);
    expect(decode).not.toHaveBeenCalled();
    expect(canvas).not.toHaveBeenCalled();
    const output = new DOMParser().parseFromString(input.template, "text/html");
    expect(output.querySelector(".katex .vlist")?.getAttribute("style")).toBe("top:-2.7em");
    expect(output.querySelector("img.ProseMirror-separator")).toBeNull();
    expect(input.template).not.toContain("blob:current-image");
    expect(input.resources).toEqual(expect.arrayContaining([{ kind: "image", assetId: "a".repeat(64), token: expect.stringMatching(/^NOTE_EXPORT_RESOURCE_/u) }]));
    expect(input.template).not.toContain("base64,");
    expect(input.template).toContain("background:#fff!important");
    expect(document.querySelector(".note-pdf-rendering")).toBeNull();
});
it("yields to input events and cleans the mounted snapshot when stopped", async () => {
    const controller = new AbortController();
    let inputHandled = false;
    setTimeout(() => { inputHandled = true; controller.abort(); }, 0);
    await expect(prepareNoteExportInput(createNotePdfSnapshot(source(), "标题"), "标题", controller.signal)).rejects.toMatchObject({ name: "AbortError" });
    expect(inputHandled).toBe(true);
    expect(document.querySelector(".note-pdf-rendering")).toBeNull();
});
it("keeps a specific error for a real unmanaged image", async () => {
    const node = document.createElement("article");
    node.innerHTML = '<img alt="real image">';
    await expect(prepareNoteExportInput(node, "标题", new AbortController().signal)).rejects.toThrow("NOTE_EXPORT_IMAGE_INVALID");
});
it("keeps streaming resource tokens distinct beyond ten resources", async () => {
    const node = document.createElement("article");
    node.innerHTML = Array.from({length:12}, () => `<figure data-asset-id="${"a".repeat(64)}"><img src="blob:current"></figure>`).join("");
    const input = await prepareNoteExportInput(node, "资源", new AbortController().signal);
    for (const resource of input.resources) expect(input.template.split(resource.token)).toHaveLength(2);
});
it("embeds declared KaTeX size faces without inventing unavailable bold font files", async () => {
    document.head.innerHTML += '<style>@font-face{font-family:KaTeX_Size1;src:url(size.woff2);font-weight:400;font-style:normal}@font-face{font-family:KaTeX_Main;src:url(bold.woff2);font-weight:bold;font-style:normal}.katex .large{font-family:KaTeX_Size1;font-weight:700}.katex .bold{font-family:KaTeX_Main;font-weight:bold}</style>';
    const node = document.createElement("article");
    node.innerHTML = '<span class="katex"><span class="large">(</span><span class="bold">x</span></span>';
    const input = await prepareNoteExportInput(node, "公式", new AbortController().signal);
    expect(input.resources.filter(resource => resource.kind === "font").map(resource => resource.name)).toContain("KaTeX_Size1-Regular.ttf");
    expect(input.resources.filter(resource => resource.kind === "font").map(resource => resource.name)).not.toContain("KaTeX_Size1-Bold.ttf");
    expect(input.resources.filter(resource => resource.kind === "font").map(resource => resource.name)).toContain("KaTeX_Main-Bold.ttf");
});
