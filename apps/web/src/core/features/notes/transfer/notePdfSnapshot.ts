/** A frozen copy of the production editor, never a second document renderer.
 * Review widgets are presentation-only; the live nodes contain the candidate.
 * The caller owns mounting and releasing this copy, including its image bytes. */
export function createNotePdfSnapshot(source: HTMLElement, title: string): HTMLElement {
  const owner = source.ownerDocument;
  const snapshot = owner.createElement("article");
  snapshot.className = "note-pdf-export block-editor note-editor-surface note-editor";
  snapshot.setAttribute("aria-hidden", "true");
  const container = owner.createElement("div");
  container.className = "bn-container bn-ariakit";
  const body = source.cloneNode(true) as HTMLElement;
  body.removeAttribute("data-note-pdf-page-id");
  // Formula and reference disclosures are document content as well as buttons.
  // Keep their rendered payload; only editing-only buttons are removed below.
  body.querySelectorAll<HTMLElement>('button:has(.katex), button[role="link"]').forEach(button => {
    const span = owner.createElement("span"); span.className = button.className;
    span.append(...button.childNodes); button.replaceWith(span);
  });
  body.querySelectorAll([
    // ProseMirror's Safari/Chrome cursor separator is an IMG without src,
    // not a note asset. Passing it to WebKit img.decode() aborts the export.
    ".ProseMirror-widget", "img.ProseMirror-separator", ".note-diff-deleted-blocks", ".note-diff-deleted-content",
    ".hbs-table-row-removed", ".hbs-table-column-removed", ".block-caption-hidden",
    ".block-media-controls", ".note-image-resize-handle", ".code-block-controls",
    ".bn-table-handle", ".bn-extend-button", "button", "input:not([type=checkbox])",
  ].join(",")).forEach(node => node.remove());
  for (const node of [body, ...body.querySelectorAll<HTMLElement>("*")]) {
    node.removeAttribute("id");
    node.removeAttribute("contenteditable");
    node.removeAttribute("tabindex");
    node.removeAttribute("data-placeholder");
    for (const name of [...node.classList]) {
      if (/^(note-diff-|hbs-|ProseMirror-selected|selectedCell)/.test(name)) node.classList.remove(name);
    }
  }
  body.querySelectorAll<HTMLElement>('[data-content-type="blockCaption"]').forEach(node => {
    if (!node.textContent?.trim()) node.closest(".block-caption-container")?.remove();
  });
  // Runtime pixel widths depend on the editor viewport. Retain the same ratio
  // on paper; captions and image wrappers must resize together.
  const sourceStyle = owner.defaultView!.getComputedStyle(source);
  const width = source.getBoundingClientRect().width - (parseFloat(sourceStyle.paddingLeft) || 0) - (parseFloat(sourceStyle.paddingRight) || 0);
  const sourceImages = [...source.querySelectorAll<HTMLElement>("figure.note-image-block")]
    .filter(node => !node.closest(".ProseMirror-widget, .note-diff-deleted-blocks, .note-diff-deleted-content"));
  [...body.querySelectorAll<HTMLElement>("figure.note-image-block")].forEach((figure, index) => {
    const original = sourceImages[index];
    const media = figure.querySelector<HTMLElement>(".note-image-resizable, .note-image-block-media");
    const originalMedia = original?.querySelector<HTMLElement>(".note-image-resizable, .note-image-block-media");
    const scale = Number(original?.dataset.scale);
    const ratio = scale > 0 ? scale / 100 : width > 0 && originalMedia ? originalMedia.getBoundingClientRect().width / width : 1;
    figure.style.gridTemplateColumns = `${Math.min(1, Math.max(.05, ratio)) * 100}%`;
    if (media) media.style.width = "100%";
    figure.querySelectorAll<HTMLElement>(".note-image-block-media").forEach(node => { node.style.width = "100%"; });
    const block = figure.closest(".bn-block");
    const caption = block?.querySelector<HTMLElement>('[data-content-type="blockCaption"]');
    if (caption) {
      caption.style.width = `${Math.min(1, Math.max(.05, ratio)) * 100}%`;
      caption.style.marginLeft = original?.dataset.alignment === "left" ? "0" : "auto";
      caption.style.marginRight = original?.dataset.alignment === "right" ? "0" : "auto";
      caption.style.marginTop = "2px";
    }
  });
  // Caption positions are measured by the editing view; discard only geometry
  // on the copy so they follow the media's paper width instead of screen pixels.
  body.querySelectorAll<HTMLElement>(".block-caption-container, .note-table-caption-slot").forEach(node => {
    node.style.removeProperty("width"); node.style.removeProperty("margin-left"); node.style.removeProperty("transform");
  });
  const sourceTables = [...source.querySelectorAll<HTMLTableElement>("table")]
    .filter(node => !node.closest(".ProseMirror-widget, .note-diff-deleted-blocks, .note-diff-deleted-content"));
  body.querySelectorAll<HTMLTableElement>("table").forEach((table, index) => {
    const columns = [...table.querySelectorAll<HTMLTableColElement>("col")];
    const widths = columns.map(col => parseFloat(col.style.width || col.getAttribute("width") || "0"));
    const total = widths.reduce((sum, width) => sum + width, 0);
    columns.forEach((col, index) => { if (total > 0) col.style.width = `${widths[index]! / total * 100}%`; col.removeAttribute("width"); });
    const measured = sourceTables[index]?.getBoundingClientRect().width || total;
    const ratio = width > 0 && measured > 0 ? Math.min(1, measured / width) : 1;
    const alignment = table.closest<HTMLElement>('[data-content-type="table"]')?.dataset.textAlignment ?? "left";
    table.style.maxWidth = "100%"; table.style.width = `${ratio * 100}%`; table.style.tableLayout = "fixed";
    table.style.marginLeft = alignment === "left" ? "0" : "auto";
    table.style.marginRight = alignment === "right" ? "0" : "auto";
    const caption = table.closest(".bn-block")?.querySelector<HTMLElement>('[data-content-type="blockCaption"]');
    if (caption) {
      caption.style.width = `${ratio * 100}%`; caption.style.marginLeft = table.style.marginLeft;
      caption.style.marginRight = table.style.marginRight; caption.style.marginTop = "10px";
    }
  });
  const documentTitle = body.querySelector<HTMLElement>('[data-content-type="documentTitle"]');
  if (documentTitle && !documentTitle.textContent?.trim()) documentTitle.textContent = title || "无标题";
  if (!documentTitle) {
    const heading = owner.createElement("h1"); heading.textContent = title || "无标题";
    snapshot.append(heading);
  }
  container.append(body); snapshot.append(container);
  return snapshot;
}
