import {COLORS_DEFAULT, type Extension} from "@blocknote/core";
import {Fragment, Slice, type Node} from "@tiptap/pm/model";
import {Plugin} from "@tiptap/pm/state";
import {NOTE_HEADING_LEVELS} from "./noteHeading";

const supportedColors = new Set(["default", ...Object.keys(COLORS_DEFAULT)]);
const maxHeadingLevel = Math.max(...NOTE_HEADING_LEVELS);

/** Keep native clipboard parsing/selection/history. Portable notes support the
 * application palette, while browsers also copy arbitrary CSS color strings. */
export function notePasteFormatting(): Extension {
  const normalize = (node: Node): Node => {
    const marks = node.marks.filter(mark => !["textColor", "backgroundColor"].includes(mark.type.name)
      || supportedColors.has(mark.attrs.stringValue));
    if (node.isText) return node.mark(marks);
    const attrs = {...node.attrs};
    if (node.type.name === "heading" && typeof attrs.level === "number") {
      attrs.level = Math.min(attrs.level, maxHeadingLevel);
    }
    for (const key of ["textColor", "backgroundColor"]) {
      if (attrs[key] !== undefined && !supportedColors.has(attrs[key])) attrs[key] = "default";
    }
    const children: Node[] = [];
    node.forEach(child => children.push(normalize(child)));
    return node.type.create(attrs, Fragment.from(children), marks);
  };
  return {key: "note-paste-formatting", prosemirrorPlugins: [new Plugin({props: {
    transformPasted(slice) {
      const children: Node[] = [];
      slice.content.forEach(node => children.push(normalize(node)));
      return new Slice(Fragment.from(children), slice.openStart, slice.openEnd);
    },
  }})]};
}
