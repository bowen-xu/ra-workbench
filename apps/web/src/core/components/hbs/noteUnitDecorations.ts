import { Plugin, PluginKey, type EditorState } from "@tiptap/pm/state";
import { Decoration, DecorationSet } from "@tiptap/pm/view";
import { projectDocumentReview, type DocumentChange, type DocumentConflict, type DocumentReviewFragment, type DocumentReviewProjection, type DocumentReviewProjector, type HbsSessionState } from "@research-assistant/core/hbs";
import type { NoteBlockV1, NoteDocumentV1 } from "@research-assistant/core/notes";
import type { NoteEditorInstance } from "../../features/notes/editor/noteEditorSchema";
import { addedTextDecorations, oldBlocksWidget } from "../../features/notes/editor/noteDiffDecorations";
import { blockToNode } from "@blocknote/core";
import { noteDocumentToBlockNote } from "../../features/notes/editor/blockNoteAdapter";
import { blockForPath, noteUnitSnapshot, pathOverlaps } from "./noteUnits";
import { tableReviewDecorations, type NoteTableComparison } from "./noteTableDecorations";
import { noteReviewDocumentFromState, sameNoteProjection } from "../../features/notes/editor/noteReviewProjection";

import { noteUnitRanges, noteAnchorPosition } from "./noteReviewRanges";
import { noteReviewGroups } from "./noteReviewGroups";
import { noteReviewGroupPresentation } from "./noteReviewGroupPresentation";
import { createNoteReviewLocation, noteReviewOldAnchor, noteReviewConflictCovers, noteReviewOldPosition, type NoteReviewLocation } from "./noteReviewProjection";

function fragmentChange(change: DocumentChange, fragment: DocumentReviewFragment): DocumentChange {
  return { ...change, path: fragment.path ?? fragment.beforePath ?? change.path,
    beforePath: fragment.beforePath ?? fragment.path ?? change.beforePath,
    before: fragment.before, after: fragment.after };
}
function projectionFor(change: DocumentChange, conflict: boolean, project: DocumentReviewProjector): DocumentReviewProjection {
  if (!conflict || !Array.isArray(change.before) && !Array.isArray(change.after)) return project(change);
  // Until all three sequence placements can be rendered together, retain the
  // complete authorized conflict. Never hide incoming behind a two-side view.
  return { decisionId: change.id, fallbackReason: "序列冲突按完整授权单元核对，保留 current 与 incoming",
    fragments: [{ beforePath: change.beforePath, path: change.path, before: change.before, after: change.after,
      positionChanged: false, contentChanged: true }] };
}
function oldAnchor(state: EditorState, value: HbsSessionState<NoteDocumentV1>, fragment: DocumentReviewFragment,
  fragments: readonly DocumentReviewFragment[]): number | undefined {
  const anchor = noteReviewOldAnchor(fragment, value.content, fragments);
  return anchor ? noteAnchorPosition(state, value.content, anchor) : undefined;
}
function defaultAnchor(state: EditorState): number { return Math.max(1, state.doc.content.size - 1); }
function markIncoming(widget: HTMLElement, blocks: readonly NoteBlockV1[]): void {
  widget.classList.add("hbs-note-incoming"); widget.setAttribute("aria-label", "incoming");
  // Empty is a real deletion proposal. Run after the production serializer's
  // microtask; an empty branch must remain visible without inventing a block.
  if (!blocks.length) queueMicrotask(() => {
    if (widget.isConnected) widget.textContent = "此范围在 incoming 中已删除。";
  });
}
export function createNoteUnitDecorations(editor: NoteEditorInstance, initial: HbsSessionState<NoteDocumentV1>,
  actions: (owner: Document, changes: readonly DocumentChange[], conflict: boolean, number: number, location?: NoteReviewLocation) => HTMLElement,
  incomingDocument: (change: DocumentConflict, current: NoteDocumentV1) => NoteDocumentV1, tableComparison?: NoteTableComparison,
  project: DocumentReviewProjector = projectDocumentReview) {
  const key = new PluginKey<{ state: HbsSessionState<NoteDocumentV1>; decorations: DecorationSet }>("hbs-unit-diff");
  const build = (state: EditorState, value: HbsSessionState<NoteDocumentV1>) => {
    // React can publish the new HBS snapshot before the parent installs it in
    // ProseMirror (and native input changes PM before publishing H). Never
    // apply row/cell coordinates across these different document generations.
    // Keep the desired snapshot in plugin state; the install transaction will
    // rebuild its decorations against the matching native document.
    if (tableComparison && !sameNoteProjection(noteReviewDocumentFromState(value.content.pageId, state.doc, value.content), value.content)) {
      return DecorationSet.empty;
    }
    const decorations: Decoration[] = [];
    const candidates = [...value.conflicts, ...value.changes.filter(change => !value.conflicts.some(conflict => noteReviewConflictCovers(conflict, change, project)))];
    const initialFragments = candidates.flatMap(change => projectionFor(change, value.conflicts.some(item => item.id === change.id), project).fragments);
    const start = (change: DocumentChange) => {
      const projection = projectionFor(change, value.conflicts.some(item => item.id === change.id), project);
      const ranges = projection.fragments.flatMap(fragment => fragment.path ? noteUnitRanges(state, value.content, fragmentChange(change, fragment)) : []);
      const anchors = projection.fragments.filter(fragment => !fragment.path).flatMap(fragment => {
        const position = oldAnchor(state, value, fragment, initialFragments); return position === undefined ? [] : [position];
      });
      return { from: Math.min(...ranges.map(range => range.from), ...anchors, defaultAnchor(state)), present: !!ranges.length };
    };
    const changes = candidates
      .sort((a, b) => {
        const left = start(a), right = start(b);
        return left.from - right.from || Number(left.present) - Number(right.present);
      });
    // Several character conflicts or property changes can belong to the same
    // rendered block/cell. Render that legal unit once and review its members
    // together instead of duplicating the complete block for each substring.
    const groups = new Map<string,DocumentChange[]>();
    const tableScopes = new Map<string, boolean>();
    const presentationChange = (change: DocumentChange): DocumentChange => {
      if (!tableComparison) return change;
      const rowsAt = change.path.lastIndexOf("rows");
      if (rowsAt < 0) return change;
      const path = change.path.slice(0, rowsAt + 1);
      if (value.conflicts.some(conflict => pathOverlaps(path, conflict.path))) return change;
      const before = blockForPath(value.baselineProjection, path), after = blockForPath(value.content, path);
      if (before?.content.kind !== "table" || after?.content.kind !== "table") return change;
      const related = changes.filter(member => pathOverlaps(path, member.path));
      const explicitGeometry = related.some(member => member.axis !== undefined
        || typeof member.path[rowsAt + 1] !== "number" || typeof member.path[rowsAt + 2] !== "number");
      // One HBS cell edit stays local even in a one-column table where daff
      // has no unchanged cell available as evidence of a row match.
      if (!explicitGeometry && related.length === 1) return change;
      // Cell-only reviews retain their existing local ranges and action hosts.
      // Daff can also reveal insert/delete pairs whose net row count is zero.
      const tableKey = JSON.stringify(path);
      let structural = tableScopes.get(tableKey);
      if (structural === undefined) {
        const correspondence = tableComparison(before.content.rows, after.content.rows);
        structural = correspondence.ambiguous || !!correspondence.columns
          || correspondence.pairs.some(pair => pair.before === undefined || pair.after === undefined)
          || explicitGeometry;
        tableScopes.set(tableKey, structural);
      }
      if (!structural) return change;
      // Group actual change IDs for one table action. This complete table is
      // only a rendering range; the original changes still authorize review.
      return { id: change.id, path, beforePath: path, kind: "node", before: before.content.rows, after: after.content.rows };
    };
    for (const change of changes) {
      const display = presentationChange(change);
      const projection = projectionFor(display, value.conflicts.some(item => item.id === change.id), project);
      const ranges = projection.fragments.flatMap(fragment => fragment.path ? noteUnitRanges(state,value.content,fragmentChange(display,fragment)) : []);
      // Shared display geometry does not authorize merging a placement decision
      // with an independent content decision on that same native node.
      const split = projection.fragments.length !== 1 || projection.fragments.some(fragment => fragment.positionChanged);
      const identity = split ? `decision:${change.id}` : ranges.length ? JSON.stringify(ranges.map(range=>[range.from,range.to])) : JSON.stringify(change.path);
      const group=groups.get(identity);if(group)group.push(change);else groups.set(identity,[change]);
    }
    const plans = [...groups.values()].map(members => {
      const decision = presentationChange(members[0]!);
      const conflicts = value.conflicts.filter(item => members.some(member => member.id === item.id));
      return { members, decision, conflicts, projection: projectionFor(decision, !!conflicts.length, project) };
    });
    const reviewGroups = noteReviewGroups(state, value, plans);
    const presentations = new Map(reviewGroups.filter((group, index) => group.firstUnit === index)
      .map(group => [group.number, noteReviewGroupPresentation(state, value, group, plans, incomingDocument)]));
    const allFragments = plans.flatMap(plan => plan.projection.fragments);
    const oldSideBudget = Math.max(allFragments.length, ...allFragments.map(fragment => fragment.placement?.before.length ?? 0)) + 1;
    const oldSide = (fragment: DocumentReviewFragment, role: number) =>
      ((noteReviewOldPosition(fragment, allFragments) ?? allFragments.indexOf(fragment)) - oldSideBudget) * 4 + role;
    // At a shared native boundary the previous live unit's controls precede
    // old fragments. Old/deleted fragments retain their source sibling order,
    // independently of which decision currently appears first in the editor.
    const liveSide = (role: number) => -(oldSideBudget + 1) * 4 + role;
    for (const [index,{members,decision,conflicts,projection}] of plans.entries()) {
      const reviewGroup = reviewGroups[index]!;
      const presentation = presentations.get(reviewGroup.number);
      if (presentation && reviewGroup.firstUnit === index) {
        const snapshot = (blocks: readonly NoteBlockV1[], incoming = false) => Decoration.widget(presentation.from, view => {
          const widget = oldBlocksWidget(editor, view.dom.ownerDocument,
            { id: presentation.key, before: blocks, after: presentation.after, afterFrom: 0 }, () => {});
          widget.dataset.hbsReviewGroup = presentation.key;
          if (incoming) markIncoming(widget, blocks);
          return widget;
        }, { side: oldSide(presentation.firstFragment, incoming ? 1 : 0),
          key: `group-${incoming ? "incoming" : "old"}:${presentation.key}:${JSON.stringify([blocks,presentation.after])}`,
          ignoreSelection: true, stopEvent: () => true });
        if (presentation.before.length) decorations.push(snapshot(presentation.before));
        if (presentation.incoming !== undefined) decorations.push(snapshot(presentation.incoming, true));
      }
      let actionTo: number | undefined, actionCell = false, actionCellText = false;
      let hasLocations = false;
      for (const [fragmentIndex, fragment] of projection.fragments.entries()) {
      const change = fragmentChange(decision, fragment);
      const ranges = fragment.path ? noteUnitRanges(state, value.content, change) : [];
      const first = ranges[0], last = ranges.at(-1);
      const previousPosition = oldAnchor(state, value, fragment, allFragments);
      const from = first?.from ?? previousPosition ?? defaultAnchor(state), to = last?.to ?? from;
      const cell = first?.node.type.name === "tableCell" || first?.node.type.name === "tableHeader";
      const cellText = cell && !change.axis;
      if (actionTo === undefined || to >= actionTo) { actionTo = to; actionCell = cell; actionCellText = cellText; }
      const beforeDocument = fragment.positionChanged && !fragment.contentChanged ? value.content : value.baselineProjection;
      const before = fragment.beforePath ? noteUnitSnapshot(beforeDocument, change, "before") : [];
      const after = fragment.path ? noteUnitSnapshot(value.content, change, "after") : [];
      const snapshotPosition = fragment.positionChanged || !fragment.path ? previousPosition ?? from : from;
      const paired = !conflicts.length && fragment.positionChanged && !!fragment.beforePath && !!fragment.path;
      const locationActions = (owner: Document, side: NoteReviewLocation["side"]) => {
        const location = createNoteReviewLocation(editor.prosemirrorView.dom, decision.id, fragment, side);
        const host = actions(owner, members, false, reviewGroup.number, location);
        host.dataset.hbsReviewDecisions = JSON.stringify(members.map(member => member.id));
        host.dataset.hbsReviewLocation = location.key; host.dataset.hbsReviewSide = location.side;
        return host;
      };
      const oldTable = before[0], currentTable = after[0];
      if (tableComparison && !conflicts.length && first && change.path.at(-1) === "rows" && !change.axis
        && oldTable?.content.kind === "table" && currentTable?.content.kind === "table") {
        decorations.push(...tableReviewDecorations(editor, first, { ...oldTable, content: oldTable.content },
          { ...currentTable, content: currentTable.content }, tableComparison));
      } else {
      if(cellText){
        const blocks=noteDocumentToBlockNote({...value.baselineProjection,blocks:before});
        const previous=blocks[0]?blockToNode(blocks[0],editor.pmSchema,editor.schema.styleSchema).firstChild?.firstChild?.firstChild:undefined;
        for(const range of ranges){
          // The native table cell stays neutral. Its real text containers own
          // the green surface, leaving the action widget as a following sibling.
          range.node.forEach((paragraph,offset)=>{
            const start=range.from+1+offset;
            decorations.push(Decoration.node(start,start+paragraph.nodeSize,{class:"note-diff-added-block hbs-note-cell-current"}));
          });
          decorations.push(...addedTextDecorations(range,previous??undefined));
        }
      }else {
        const previous=noteDocumentToBlockNote({...value.baselineProjection,blocks:before}).map(block=>blockToNode(block,editor.pmSchema,editor.schema.styleSchema));
        for (const range of ranges) {
          decorations.push(Decoration.node(range.from, range.to, { class: `note-diff-added-block${presentation && range.from !== presentation.firstCurrent ? " hbs-note-group-continuation" : ""}`,
            "data-hbs-review-decision": decision.id, ...(presentation ? { "data-hbs-review-group": presentation.key } : {}),
            ...(presentation?.before.length && range.from === presentation.firstCurrent ? { "data-block-review-anchor": range.node.attrs.id } : {}),
            ...(fragment.positionChanged ? { "data-hbs-review-placement": "after" } : {}) }));
          if (fragment.contentChanged) decorations.push(...addedTextDecorations(range,previous.find(node=>node.attrs.id===range.node.attrs.id)));
        }
      }
      const snapshot = (blocks: typeof before, incoming = false) => Decoration.widget(snapshotPosition + (cell ? 1 : 0), view => {
        const widget = oldBlocksWidget(editor, view.dom.ownerDocument, { id: change.id, before: blocks, after, afterFrom: 0 }, () => {},
          paired && !incoming ? host => { host.append(locationActions(view.dom.ownerDocument, "before")); } : undefined,cellText?"tableCell":undefined);
        widget.dataset.hbsReviewDecision = decision.id;
        if (fragment.positionChanged) { widget.dataset.hbsReviewPlacement = "before"; widget.setAttribute("aria-label", "原位置"); }
        if (incoming) markIncoming(widget, blocks);
        return widget;
      }, { side: oldSide(fragment, incoming ? 1 : 0), key: `${incoming ? "incoming" : "old"}:${change.id}:${fragmentIndex}:${JSON.stringify(blocks)}`, ignoreSelection: true, stopEvent: () => true });
      if (!presentation && before.length) decorations.push(snapshot(before));
      if (!presentation && conflicts.length) decorations.push(snapshot(noteUnitSnapshot(conflicts.reduce((current,conflict)=>incomingDocument(conflict,current),value.content), change, "after"), true));
      }
      if (paired) {
        hasLocations = true;
        decorations.push(Decoration.widget(cell ? to - 1 : to, view => locationActions(view.dom.ownerDocument, "after"), {
          side: liveSide(3), key: `location-actions:${decision.id}:${fragmentIndex}`, ignoreSelection: true, stopEvent: () => true,
        }));
      }
      }
      if (hasLocations || reviewGroup.lastUnit !== index) continue;
      const to = actionTo ?? defaultAnchor(state), cell = actionCell, cellText = actionCellText;
      const deletion = projection.fragments.length === 1 && !projection.fragments[0]?.path ? projection.fragments[0] : undefined;
      if (projection.fallbackReason) decorations.push(Decoration.widget(cell ? to - 1 : to, view => {
        const reason = view.dom.ownerDocument.createElement("p"); reason.className = "hbs-review-projection-reason";
        reason.contentEditable = "false"; reason.textContent = projection.fallbackReason!; return reason;
      }, { side: deletion ? oldSide(deletion, 2) : liveSide(2), key: `projection-reason:${decision.id}:${projection.fallbackReason}`, ignoreSelection: true }));
      decorations.push(Decoration.widget(cell ? to - 1 : to, view => {
        const host=actions(view.dom.ownerDocument, reviewGroup.members,!!conflicts.length,reviewGroup.number);
        host.dataset.hbsReviewDecisions = JSON.stringify(reviewGroup.members.map(member => member.id));
        if(cellText)host.classList.add("hbs-note-cell-actions");
        return host;
      }, {
        side: deletion ? oldSide(deletion, 3) : liveSide(3), key: `actions:${reviewGroup.members.map(item=>item.id).join("|")}:${!!conflicts.length}:${reviewGroup.number}`, ignoreSelection: true, stopEvent: () => true,
      }));
    }
    return DecorationSet.create(state.doc, decorations);
  };
  return { key, plugin: new Plugin({ key, state: {
    init: (_, state) => ({ state: initial, decorations: build(state, initial) }),
    apply: (transaction, previous, _old, state) => {
      const next = transaction.getMeta(key) as HbsSessionState<NoteDocumentV1> | undefined;
      return next || transaction.docChanged ? { state: next ?? previous.state, decorations: build(state, next ?? previous.state) } : previous;
    },
  }, props: { decorations: state => key.getState(state)?.decorations ?? DecorationSet.empty } }) };
}
