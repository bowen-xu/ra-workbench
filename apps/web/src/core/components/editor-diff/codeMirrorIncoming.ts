import { ChangeSet, RangeSet, StateField, Text, type EditorState } from "@codemirror/state";
import { getChunks, getOriginalDoc } from "@codemirror/merge";
import { Decoration, EditorView, GutterMarker, gutterLineClass, gutterWidgetClass, WidgetType, type DecorationSet } from "@codemirror/view";
import { textChanges, textChunkResolution, textDiffResolution } from "./textDiffCommands";

/** Replacement coordinates refer to the current editable document, never the baseline. */
export interface TextEditorIncoming {
  readonly id: string;
  readonly ids?: readonly string[];
  readonly from: number;
  readonly to: number;
  readonly text: string;
  readonly base?: string;
  readonly document?: string;
}
export interface IncomingResolution {
  readonly ids: readonly string[];
  readonly action: "incoming" | "both" | "keep" | "undo";
  readonly value: string;
  readonly baseline: string;
}
export interface IncomingGroup {
  readonly id: string;
  readonly ids: readonly string[];
  readonly from: number;
  readonly to: number;
  readonly incoming: string;
  readonly original: string;
  readonly chunk: boolean;
  readonly incomingHost: HTMLElement;
  readonly originalHost: HTMLElement;
  readonly actionsHost: HTMLElement;
}

/** Group real merge conflicts by the editor's actual review chunks. Conflicts in
 * ordinary input have no pending chunk yet, so retain their containing lines. */
export function incomingGroups(state: EditorState, incoming: readonly TextEditorIncoming[], hosts: Map<string, HTMLElement>): IncomingGroup[] {
  const groups = new Map<string, { from: number; to: number; chunk: boolean; items: TextEditorIncoming[] }>();
  for (const item of incoming) {
    const chunk = getChunks(state)?.chunks.find(chunk => item.from >= chunk.fromB && item.to <= chunk.toB);
    const from = chunk?.fromB ?? state.doc.lineAt(item.from).from;
    const to = Math.min(state.doc.length, chunk?.toB ?? state.doc.lineAt(item.to > item.from ? item.to - 1 : item.to).to);
    const key = `${from}:${to}`;
    const group = groups.get(key) ?? { from, to, chunk: !!chunk, items: [] };
    group.items.push(item); groups.set(key, group);
  }
  const mount = (id: string, className: string) => {
    let host = hosts.get(id);
    if (!host) { host = document.createElement("div"); host.className = className; hosts.set(id, host); }
    return host;
  };
  return [...groups.values()].map(group => {
    const ids = group.items.flatMap(item => item.ids ?? [item.id]), id = ids.join("|");
    const current = state.doc.slice(group.from, group.to);
    const changes = ChangeSet.of(group.items.map(item => ({ from: item.from - group.from, to: item.to - group.from,
      insert: Text.of(item.text.split("\n")) })), current.length);
    const source = group.items[0]?.document;
    const sourceMapping = source === undefined ? undefined : textChanges(state.doc.toString(), source);
    const baseline = getOriginalDoc(state), baselineMapping = textChanges(state.doc.toString(), baseline.toString());
    return { id, ids, from: group.from, to: group.to, chunk: group.chunk,
      incoming: source === undefined ? changes.apply(current).toString()
        : source.slice(sourceMapping!.mapPos(group.from, -1), sourceMapping!.mapPos(group.to, 1)),
      // Red old and Undo share the same B′ slice, including unsaved ordinary
      // input. Never synthesize red from current plus the persisted B.
      original: baseline.sliceString(baselineMapping.mapPos(group.from, -1), baselineMapping.mapPos(group.to, 1)),
      incomingHost: mount(`${id}:incoming`, "text-editor-incoming"),
      originalHost: mount(`${id}:original`, "text-editor-conflict-original"),
      actionsHost: mount(`${id}:actions`, "editor-diff-chunk-actions writing-diff-chunk-actions"),
    };
  });
}

class IncomingWidget extends WidgetType {
  constructor(readonly element: HTMLElement, readonly kind: "incoming" | "original" | "actions") { super(); }
  override eq(other: IncomingWidget): boolean { return this.element === other.element && this.kind === other.kind; }
  override toDOM(): HTMLElement { this.element.contentEditable = "false"; return this.element; }
  override ignoreEvent(): boolean { return true; }
}

class ConflictGutterMarker extends GutterMarker {
  constructor(override readonly elementClass: string) { super(); }
}
const incomingMarker = new ConflictGutterMarker("text-editor-incoming-gutter");
const originalMarker = new ConflictGutterMarker("text-editor-conflict-original-gutter");
const currentMarker = new ConflictGutterMarker("text-editor-current-conflict-gutter");

export function incomingDecorations(state: EditorState, groups: readonly IncomingGroup[]) {
  const initial = Decoration.set(groups.flatMap(group => {
    const decorations = [
      // Upstream deletion widgets have side -1. Side 0 follows red old, before
      // editable green current, without putting incoming into the document.
      Decoration.widget({ widget: new IncomingWidget(group.incomingHost, "incoming"), block: true, side: 0 }).range(group.from),
      Decoration.widget({ widget: new IncomingWidget(group.actionsHost, "actions"), block: true, side: 1 }).range(state.doc.lineAt(group.to > group.from ? group.to - 1 : group.to).to),
    ];
    if (!group.chunk) decorations.push(Decoration.widget({ widget: new IncomingWidget(group.originalHost, "original"), block: true, side: -1 }).range(group.from));
    if (!group.chunk) for (let line = state.doc.lineAt(group.from).number; line <= state.doc.lineAt(group.to).number; line++) {
      decorations.push(Decoration.line({ class: "text-editor-current-conflict" }).range(state.doc.line(line).from));
    }
    return decorations;
  }), true);
  const markers = RangeSet.of(groups.filter(group => !group.chunk).flatMap(group => {
    const lines = [];
    for (let line = state.doc.lineAt(group.from).number; line <= state.doc.lineAt(group.to).number; line++) lines.push(currentMarker.range(state.doc.line(line).from));
    return lines;
  }), true);
  const field = StateField.define<{ readonly decorations: DecorationSet; readonly markers: RangeSet<GutterMarker> }>({
    create: () => ({ decorations: initial, markers }),
    update: (value, transaction) => ({ decorations: value.decorations.map(transaction.changes), markers: value.markers.map(transaction.changes) }),
    provide: field => [EditorView.decorations.from(field, value => value.decorations), gutterLineClass.from(field, value => value.markers)],
  });
  // These facets decorate the existing merge gutter; adding another gutter()
  // would create a second column. CSS scopes paint to .cm-changeGutter so the
  // same marker classes on line-number cells do not color the number column.
  return [field, gutterWidgetClass.of((_view, widget) => widget instanceof IncomingWidget
    ? widget.kind === "incoming" ? incomingMarker : widget.kind === "original" ? originalMarker : null : null)];
}

export function resolveIncomingGroup(state: EditorState, group: IncomingGroup, action: IncomingResolution["action"]): IncomingResolution {
  let value = state.doc.toString(), baseline = getOriginalDoc(state).toString();
  if (action === "incoming" || action === "both") {
    const current = state.doc.sliceString(group.from, group.to);
    const replacement = action === "incoming" ? group.incoming
      : group.incoming === current ? current : group.incoming + (group.incoming.endsWith("\n") || !group.incoming || !current ? "" : "\n") + current;
    value = value.slice(0, group.from) + replacement + value.slice(group.to);
  } else if (group.chunk) {
    const transaction = textChunkResolution(state, group.from, action);
    if (transaction) {
      const next = state.update(transaction).state;
      value = next.doc.toString(); baseline = getOriginalDoc(next).toString();
    }
  } else if (action === "undo") {
    const mapping = textChanges(value, baseline);
    baseline = baseline.slice(0, mapping.mapPos(group.from, -1)) + group.original + baseline.slice(mapping.mapPos(group.to, 1));
    value = value.slice(0, group.from) + group.original + value.slice(group.to);
  }
  return { ids: group.ids, action, value, baseline };
}

export function resolveIncomingReview(state: EditorState, groups: readonly IncomingGroup[], action: "keep" | "undo"): IncomingResolution {
  const transaction = textDiffResolution(state, action), next = transaction ? state.update(transaction).state : state;
  const restore = (target: string) => {
    const mapping = textChanges(state.doc.toString(), target);
    return ChangeSet.of(groups.filter(group => !group.chunk).map(group => ({
      from: mapping.mapPos(group.from, -1), to: mapping.mapPos(group.to, 1), insert: Text.of(group.original.split("\n")),
    })), target.length).apply(Text.of(target.split("\n"))).toString();
  };
  return { ids: groups.flatMap(group => group.ids), action,
    value: action === "undo" ? restore(next.doc.toString()) : next.doc.toString(),
    baseline: action === "undo" ? restore(getOriginalDoc(next).toString()) : getOriginalDoc(next).toString(),
  };
}
