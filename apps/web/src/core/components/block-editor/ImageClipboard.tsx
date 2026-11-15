import {useEffect} from "react";
import type {EditorView} from "@tiptap/pm/view";
import {NodeSelection} from "@tiptap/pm/state";
import {BlockSetSelection} from "./BlockSetSelection";
import {notify} from "../../services/notifications";

export function isImageClipboardHTML(html: string): boolean {
  if (!html.includes("managedImage")) return false;
  const fragment = new DOMParser().parseFromString(html, "text/html");
  // A system clipboard may remove our marker or rewrite img.src. The durable
  // block identity, rather than its temporary pixel URL, selects raw parsing.
  const resource = fragment.querySelector('[data-content-type="managedImage"][data-asset-id]');
  return !!resource?.getAttribute("data-asset-id")?.trim();
}

/** When only a standard image fragment survives, use the host's existing
 * durable File importer. Never request file:/remote URLs or discard prose. */
export function embeddedImageClipboardFile(html: string): File | undefined {
  if (!/data:image\/png;base64,/iu.test(html)) return;
  const fragment = new DOMParser().parseFromString(html, "text/html");
  const images = fragment.body.querySelectorAll("img");
  if (images.length !== 1 || fragment.body.textContent?.trim()) return;
  const match = /^data:image\/png;base64,([\s\S]*)$/iu.exec(images[0]!.getAttribute("src") ?? "");
  if (!match) return;
  try {
    const bytes = Uint8Array.from(atob(match[1]!), character => character.charCodeAt(0));
    if (!bytes.length) throw new Error("empty PNG");
    return new File([bytes], "image.png", {type:"image/png"});
  } catch (cause) {
    throw new Error("NOTE_IMAGE_PASTE_DECODE_FAILED：剪贴板内嵌图片数据无效。", {cause});
  }
}

function selectedImage(view: EditorView): HTMLImageElement | undefined {
  const selection = view.state.selection;
  if (!(selection instanceof NodeSelection) && !(selection instanceof BlockSetSelection && selection.blockIds.length === 1)) return;
  const root = selection instanceof NodeSelection ? selection.node : selection.content().content.firstChild;
  const node = root?.type.name === "blockContainer" ? root.firstChild : root;
  if (node?.type.name !== "managedImage" || !node.attrs.assetId) return;
  const figure = [...view.dom.querySelectorAll<HTMLElement>("figure.note-image-block")]
    .find(element => element.dataset.assetId === node.attrs.assetId);
  return figure?.querySelector("img") ?? undefined;
}

/** Copy original pixels, independent of display scale and selection paint. */
function pngSnapshot(image: HTMLImageElement): Promise<Blob> {
  return new Promise((resolve, reject) => {
    if (!image.complete || !image.naturalWidth || !image.naturalHeight) {
      reject(new Error("NOTE_IMAGE_COPY_NOT_READY：图片尚未加载完成。")); return;
    }
    const canvas = document.createElement("canvas");
    canvas.width = image.naturalWidth; canvas.height = image.naturalHeight;
    const context = canvas.getContext("2d");
    if (!context) {reject(new Error("NOTE_IMAGE_COPY_CANVAS_UNAVAILABLE：无法读取图片像素。")); return;}
    context.drawImage(image, 0, 0);
    canvas.toBlob(blob => blob ? resolve(blob) : reject(new Error("NOTE_IMAGE_COPY_ENCODE_FAILED：图片无法编码。")), "image/png");
  });
}

function embeddedHTML(html: string, png: Blob): Promise<Blob> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error("NOTE_IMAGE_COPY_ENCODE_FAILED：无法生成可粘贴的图片内容。"));
    reader.onload = () => {
      const fragment = document.createElement("div"); fragment.innerHTML = html;
      const resource = fragment.querySelector('[data-content-type="managedImage"]') ?? fragment;
      resource.setAttribute("data-note-image-clipboard", "1");
      let image = resource.querySelector("img");
      if (!image) {image = document.createElement("img"); resource.prepend(image);}
      image.src = String(reader.result);
      resolve(new Blob([fragment.innerHTML], {type: "text/html"}));
    };
    reader.readAsDataURL(png);
  });
}

/** Reuses native rich-block serialization, with one clipboard writer.
 * Request access synchronously with promised PNG/HTML, then consume the copy
 * event's payload so it cannot race this write when image encoding completes. */
export function ImageClipboard({editor}: {readonly editor: {readonly prosemirrorView: EditorView; readonly domElement: HTMLElement | undefined}}): null {
  useEffect(() => {
    const workspace = editor.domElement?.closest(".block-editor");
    if (!workspace) return;
    let active = true, generation = 0;
    const copy = (event: Event) => {
      const current = ++generation, data = (event as ClipboardEvent).clipboardData;
      const image = selectedImage(editor.prosemirrorView);
      if (!image || !data) return;
      try {
        if (!navigator.clipboard?.write || typeof ClipboardItem === "undefined") throw new Error("NOTE_IMAGE_COPY_UNSUPPORTED：当前环境不支持复制图片数据。");
        const png = pngSnapshot(image);
        // A copy event may silently discard custom MIME (notably in WebKit).
        // Its text/html is the external presentation, not the block document:
        // it can contain async preview placeholders and omit schema attributes.
        // Regenerate the native rich representation from the actual selection,
        // just as BlockNote does, instead of treating external HTML as raw data.
        const html = data.getData("blocknote/html")
          || editor.prosemirrorView.serializeForClipboard(editor.prosemirrorView.state.selection.content()).dom.innerHTML;
        const htmlData = png.then(blob => embeddedHTML(html, blob));
        // A denied clipboard request may reject before consuming its data.
        // Keep encoding failures handled even in that path.
        void png.catch(() => {}); void htmlData.catch(() => {});
        const item = new ClipboardItem({"image/png": png, "text/html": htmlData,
          "text/plain": new Blob([data.getData("text/plain")], {type: "text/plain"})});
        const request = navigator.clipboard.write([item]);
        // WebKit commits nonempty clipboardData after copy handlers return.
        // That changes the pasteboard version while this async writer is
        // encoding PNG/HTML, causing ItemWriter::didSetAllData to reject it.
        // Capture all formats above, then let only this request publish them.
        // Keep these operations in the copy gesture, without await or timers.
        // https://github.com/WebKit/WebKit/blob/main/Source/WebCore/Modules/async-clipboard/Clipboard.cpp
        event.preventDefault();
        data.clearData();
        void request.catch(reason => {
          if (active && current === generation && reason?.name !== "AbortError") notify.error(`NOTE_IMAGE_COPY_FAILED：${String(reason)}`);
        });
      } catch (reason) {if (active) notify.error(`NOTE_IMAGE_COPY_FAILED：${String(reason)}`);}
    };
    workspace.addEventListener("copy", copy);
    return () => {active = false; workspace.removeEventListener("copy", copy);};
  }, [editor]);
  return null;
}
