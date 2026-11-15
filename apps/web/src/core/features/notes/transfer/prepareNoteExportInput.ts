import { type NoteExportInput, type NoteExportResource, checkExportCancelled, exportCanvasCss, yieldExport } from "./noteExportInput";
/** Capture browser-owned layout inputs only. Large binary resources stay out
 * of this document and are resolved by a cancellable background adapter. */
export async function prepareNoteExportInput(snapshot: HTMLElement, title: string, signal: AbortSignal): Promise<NoteExportInput> {
    checkExportCancelled(signal);
    const owner = snapshot.ownerDocument;
    snapshot.classList.add("note-pdf-rendering");
    owner.body.append(snapshot);
    const resources: NoteExportResource[] = [];
    const prefix = `NOTE_EXPORT_RESOURCE_${crypto.randomUUID().replaceAll("-", "")}_`;
    // A terminator prevents resource 1 from also matching resources 10, 11, …
    // in the native streaming replacement adapter.
    const token = () => `${prefix}${resources.length}_END`;
    try {
        const failure = snapshot.querySelector(".note-image-block-placeholder, .note-image-block-error");
        if (failure)
            throw new Error("NOTE_EXPORT_IMAGE_UNAVAILABLE：图片尚未加载完成或资源无法读取。");
        const styles: string[] = [];
        const availableMathFaces: {
            name: string;
            family: string;
            weight: 400 | 700;
            style: "normal" | "italic";
        }[] = [];
        for (const sheet of owner.styleSheets) {
            if (sheet.disabled)
                continue;
            try {
                for (const rule of sheet.cssRules) {
                    if (rule.type !== CSSRule.FONT_FACE_RULE) {
                        styles.push(rule.cssText);
                        continue;
                    }
                    const face = rule as CSSFontFaceRule;
                    const family = face.style.getPropertyValue("font-family").trim().replace(/["']/gu, "");
                    if (!/^KaTeX_[A-Za-z]+[1-4]?$/u.test(family))
                        continue;
                    const declaredWeight = face.style.getPropertyValue("font-weight");
                    const weight = declaredWeight === "bold" || Number(declaredWeight) >= 600 ? 700 : 400;
                    const italic = face.style.getPropertyValue("font-style") === "italic";
                    const name = `${family}-${weight === 700 ? italic ? "BoldItalic" : "Bold" : italic ? "Italic" : "Regular"}.ttf`;
                    availableMathFaces.push({ name, family, weight, style: italic ? "italic" : "normal" });
                }
            }
            catch (cause) {
                throw new Error(`NOTE_EXPORT_STYLES_UNAVAILABLE：${cause instanceof Error ? cause.message : String(cause)}`, { cause });
            }
            await yieldExport(signal);
        }
        const fonts = new Map<string, {
            family: string;
            weight: 400 | 700;
            style: "normal" | "italic";
        }>();
        fonts.set("ResearchAssistantSans-Regular.ttf", { family: "ResearchAssistantSans", weight: 400, style: "normal" });
        fonts.set("ResearchAssistantSans-Bold.ttf", { family: "ResearchAssistantSans", weight: 700, style: "normal" });
        const math = snapshot.querySelectorAll<HTMLElement>(".katex, .katex *");
        const mathFamilies = new Set<string>();
        for (let index = 0; index < math.length; index++) {
            if (index % 100 === 0)
                await yieldExport(signal);
            const computed = getComputedStyle(math[index]!);
            const family = computed.fontFamily.split(",")[0]!.trim().replace(/["']/gu, "");
            if (/^KaTeX_[A-Za-z]+[1-4]?$/u.test(family))
                mathFamilies.add(family);
        }
        // Preserve the family's declared faces and let the browser perform its
        // native weight/style fallback (Size1..4, for example, only have Regular).
        for (const face of availableMathFaces)
            if (mathFamilies.has(face.family))
                fonts.set(face.name, face);
        for (const [name, font] of fonts) {
            const id = token();
            resources.push({ token: id, kind: "font", name });
            styles.push(`@font-face{font-family:${JSON.stringify(font.family)};font-weight:${font.weight};font-style:${font.style};src:url(${id}) format("truetype");}`);
        }
        const images = snapshot.querySelectorAll<HTMLImageElement>("img");
        for (let index = 0; index < images.length; index++) {
            if (index % 20 === 0)
                await yieldExport(signal);
            const image = images[index]!;
            const assetId = image.closest<HTMLElement>("[data-asset-id]")?.dataset.assetId;
            if (!assetId || !/^[a-f0-9]{64}$/iu.test(assetId))
                throw new Error("NOTE_EXPORT_IMAGE_INVALID：图片缺少受管理的资源 ID。");
            const id = token();
            resources.push({ token: id, kind: "image", assetId });
            // Keep resource placeholders out of a live src attribute: otherwise a
            // browser could start a pointless request while the snapshot is mounted.
            image.removeAttribute("srcset");
            image.removeAttribute("src");
            image.setAttribute("data-note-export-resource", id);
        }
        styles.push(exportCanvasCss);
        snapshot.style.position = "static";
        snapshot.style.left = "auto";
        snapshot.style.top = "auto";
        const output = owner.implementation.createHTMLDocument();
        output.title = title;
        output.documentElement.lang = "zh-CN";
        const charset = output.createElement("meta");
        charset.setAttribute("charset", "utf-8");
        output.head.prepend(charset);
        const policy = output.createElement("meta");
        policy.httpEquiv = "Content-Security-Policy";
        policy.content = "default-src 'none'; img-src data:; font-src data:; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'";
        output.head.append(policy);
        const style = output.createElement("style");
        style.textContent = styles.join("\n").replace(/<\/style/giu, "<\\/style");
        output.head.append(style);
        snapshot.removeAttribute("aria-hidden");
        snapshot.querySelectorAll("script,iframe,object,embed,link,meta,base").forEach(node => node.remove());
        const elements = [snapshot, ...snapshot.querySelectorAll("*")];
        for (let index = 0; index < elements.length; index++) {
            if (index % 200 === 0)
                await yieldExport(signal);
            for (const attribute of [...elements[index]!.attributes])
                if (/^on/iu.test(attribute.name))
                    elements[index]!.removeAttribute(attribute.name);
        }
        checkExportCancelled(signal);
        output.body.append(snapshot);
        const html = output.documentElement.outerHTML.replace(/data-note-export-resource="(NOTE_EXPORT_RESOURCE_[A-Za-z0-9_]+)"/gu, 'src="$1"');
        return { template: `<!doctype html>\n${html}`, resources };
    }
    finally {
        snapshot.remove();
    }
}
