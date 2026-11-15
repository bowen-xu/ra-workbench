import type { NoteExportInput, NoteExportResource } from "./noteExportInput";
function imageMime(bytes: Uint8Array): string {
    const text = new TextDecoder().decode(bytes.subarray(0, 4096));
    if (/<svg(?:\s|>)/iu.test(text))
        return "image/svg+xml";
    if (bytes[0] === 137 && bytes[1] === 80 && bytes[2] === 78 && bytes[3] === 71)
        return "image/png";
    if (bytes[0] === 255 && bytes[1] === 216 && bytes[2] === 255)
        return "image/jpeg";
    if (text.startsWith("GIF8"))
        return "image/gif";
    if (text.startsWith("RIFF") && text.slice(8, 12) === "WEBP")
        return "image/webp";
    // The browser image decoder sniffs other raster formats. SVG needs its
    // explicit type, while original image bytes are never re-encoded as PNG.
    return "application/octet-stream";
}
/** Worker-only binary assembly. Return a Blob so posting the result does not
 * clone a huge UTF-8 string or expand bytes into a UI-thread number array. */
export async function buildNoteHtmlBlob(input: NoteExportInput, read: (resource: NoteExportResource) => Promise<Uint8Array>, check: () => void = () => { }): Promise<Blob> {
    const ranges = input.resources.map(resource => {
        const start = input.template.indexOf(resource.token);
        if (start < 0 || input.template.indexOf(resource.token, start + resource.token.length) >= 0)
            throw new Error("NOTE_EXPORT_RESOURCE_INVALID：资源占位标识缺失或不唯一。");
        return { start, end: start + resource.token.length, resource };
    }).sort((a, b) => a.start - b.start);
    const parts: BlobPart[] = [];
    let cursor = 0;
    let size = input.template.length;
    for (const range of ranges) {
        check();
        if (range.start < cursor)
            throw new Error("NOTE_EXPORT_RESOURCE_INVALID：资源占位标识重叠。");
        parts.push(input.template.slice(cursor, range.start));
        const bytes = await read(range.resource);
        check();
        size += Math.ceil(bytes.length / 3) * 4 + 128;
        if (size > 256 * 1024 * 1024)
            throw new Error("NOTE_EXPORT_TOO_LARGE：导出文件超过 256 MB。");
        parts.push(`data:${range.resource.kind === "font" ? "font/ttf" : imageMime(bytes)};base64,`);
        // A multiple of three preserves base64 concatenation; small argument
        // batches also stay below WebKit's Function argument limit.
        for (let offset = 0; offset < bytes.length; offset += 12 * 1024) {
            check();
            parts.push(btoa(String.fromCharCode(...bytes.subarray(offset, offset + 12 * 1024))));
        }
        cursor = range.end;
    }
    check();
    parts.push(input.template.slice(cursor));
    return new Blob(parts, { type: "text/html;charset=utf-8" });
}
