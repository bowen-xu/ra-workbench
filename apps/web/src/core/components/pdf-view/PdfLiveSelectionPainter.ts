import type { PdfLiveCharacterSelection } from "./pdfCharacterSelectionController";

interface PageHost { readonly pageIndex: number; readonly element: HTMLElement }

/** Transient page content, separate from React-owned committed selections.
 * Called in the input controller's frame so live paint has no React commit hop. */
export class PdfLiveSelectionPainter {
  private hosts: readonly PageHost[] = [];
  private layers = new Map<HTMLElement, HTMLElement>();
  private selection: PdfLiveCharacterSelection | undefined;

  setHosts(hosts: readonly PageHost[]): void {
    if (hosts.length === this.hosts.length && hosts.every((host, index) =>
      host.pageIndex === this.hosts[index]?.pageIndex && host.element === this.hosts[index]?.element)) return;
    for (const [host, layer] of this.layers) {
      if (!hosts.some(candidate => candidate.element === host)) { layer.remove(); this.layers.delete(host); }
    }
    this.hosts = hosts;
    this.render(this.selection);
  }

  render(selection: PdfLiveCharacterSelection | undefined): void {
    this.selection = selection;
    for (const host of this.hosts) {
      const rects = selection?.segments.filter(segment => segment.pageIndex === host.pageIndex)
        .flatMap(segment => segment.rects) ?? [];
      let layer = this.layers.get(host.element);
      if (!rects.length) { layer?.remove(); this.layers.delete(host.element); continue; }
      if (!layer) {
        layer = host.element.ownerDocument.createElement("div");
        layer.className = "pdf-live-selection-layer";
        layer.setAttribute("aria-hidden", "true");
        this.layers.set(host.element, layer);
      }
      if (layer.parentElement !== host.element) host.element.append(layer);
      const children = rects.map(rect => {
        const node = host.element.ownerDocument.createElement("span");
        node.className = "pending-selection-highlight";
        Object.assign(node.style, { left: `${rect.x * 100}%`, top: `${rect.y * 100}%`,
          width: `${rect.width * 100}%`, height: `${rect.height * 100}%` });
        return node;
      });
      layer.replaceChildren(...children);
    }
  }

  clear(): void {
    this.selection = undefined;
    for (const layer of this.layers.values()) layer.remove();
    this.layers.clear();
  }
}
