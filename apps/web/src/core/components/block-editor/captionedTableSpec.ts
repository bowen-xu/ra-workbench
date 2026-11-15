import {createTableBlockSpec} from "@blocknote/core";
import {BLOCK_CAPTION} from "./captionBlockProjection";
import type {PortableEditorBlock} from "@research-assistant/core/notes";

const upstream = createTableBlockSpec();

/** Keep the native table/cell/resize view. The media toolbar uses a content
 * slot outside its contentDOM; captions live in native child text blocks. */
export const captionedTableSpec = {
  ...upstream,
  config: {...upstream.config, propSchema: {...upstream.config.propSchema, caption: {default: ""},
    textAlignment: {default: "left" as const, values: ["left", "center", "right"] as const}}},
  implementation: {...upstream.implementation,
    render(block: PortableEditorBlock, editor: Parameters<typeof upstream.implementation.render>[1]) {
      const output = upstream.implementation.render.call(this, block, editor);
      // Full HTML already renders the native child caption. The portable-only
      // representation continues to export its legacy table caption element.
      if (block.children?.some(child => String(child.type) === BLOCK_CAPTION)) {
        const table = output.dom.querySelector("table");
        table?.setAttribute("data-caption", String(block.props?.caption ?? ""));
        table?.querySelector(":scope > caption")?.remove();
      }
      return output;
    }, node: upstream.implementation.node.extend({
    addAttributes() {
      return {...this.parent?.(), textAlignment: {default: "left", keepOnSplit: true,
        parseHTML: element => element.getAttribute("data-text-alignment")
          ?? element.closest<HTMLElement>("[data-text-alignment]")?.dataset.textAlignment ?? "left",
        renderHTML: attributes => ({"data-text-alignment": attributes.textAlignment}),
      }, caption: {default: "", keepOnSplit: true,
        parseHTML: element => element.getAttribute("data-caption") ?? element.closest<HTMLElement>("[data-caption]")?.dataset.caption
          ?? element.querySelector(":scope > caption")?.textContent ?? "",
        renderHTML: attributes => attributes.caption ? {"data-caption": attributes.caption} : {},
      }};
    },
    renderHTML(props) {
      const output = this.parent?.(props);
      if (output && typeof output === "object" && "dom" in output && output.dom instanceof HTMLElement) {
        const table = output.dom.querySelector("table");
        if (table && props.node.attrs.caption) {
          const caption = document.createElement("caption");
          caption.textContent = String(props.node.attrs.caption);
          caption.style.captionSide = "bottom";
          table.prepend(caption);
        }
      }
      return output!;
    },
    addNodeView() {
      const createView = this.parent?.();
      return props => {
        if (!createView) throw new Error("table_caption_view_unavailable：原生表格视图不可用。");
        const view = createView(props);
        const surface = document.createElement("div");
        surface.className = "note-table-block-surface";
        // TableView retains its own table, contentDOM and column-resize engine.
        // The surrounding content slots remain outside the clipped cell view.
        surface.append(...Array.from(view.dom.childNodes));
        view.dom.classList.add("note-table-block");
        view.dom.appendChild(surface);
        const controls = document.createElement("div");
        controls.className = "note-table-controls-slot";
        controls.contentEditable = "false";
        const media = surface.querySelector<HTMLElement>(".tableWrapper");
        if (!media) throw new Error("table_media_view_unavailable：原生表格容器不可用。");
        media.classList.add("block-media-surface");
        media.appendChild(controls);
        const align = (value: unknown) => {
          view.dom.setAttribute("data-text-alignment", value === "center" || value === "right" ? value : "left");
        };
        align(props.node.attrs.textAlignment);
        const update = view.update?.bind(view);
        view.update = (...args) => {
          if (!update?.(...args)) return false;
          align(args[0].attrs.textAlignment); return true;
        };
        const stopEvent = view.stopEvent?.bind(view);
        view.stopEvent = event => controls.contains(event.target as Node)
          || (stopEvent?.(event) ?? false);
        const ignoreMutation = view.ignoreMutation?.bind(view);
        view.ignoreMutation = mutation => controls.contains(mutation.target)
          || mutation.target === surface || (ignoreMutation?.(mutation) ?? false);
        return view;
      };
    },
  })},
};
