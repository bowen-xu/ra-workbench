import { isHistoryTransaction } from "@tiptap/pm/history";
import { Plugin, PluginKey, type Transaction } from "@tiptap/pm/state";
import { Step, StepResult, type Mappable } from "@tiptap/pm/transform";
import type { Node } from "@tiptap/pm/model";
import type { HbsHistoryEntry, HbsSession } from "@research-assistant/core/hbs";
import type { NoteDocumentV1 } from "@research-assistant/core/notes";
import { noteReviewDocumentFromState } from "../../features/notes/editor/noteReviewProjection";
import {BLOCK_TEXT_INTENT, type BlockTextIntent} from "../block-editor/blockTextEditing";
import {projectNoteBlockTextIntent} from "../../features/notes/editor/noteBlockTextIntent";

/** A reversible state step lives in PM's existing history event. It has no
 * document positions and therefore survives unrelated block/cell edits. */
class SessionStep extends Step {
  constructor(readonly entry: HbsHistoryEntry<NoteDocumentV1>, readonly direction: "before" | "after" = "after") { super(); }
  apply(doc: Node): StepResult { return StepResult.ok(doc); }
  invert(): Step { return new SessionStep(this.entry, this.direction === "before" ? "after" : "before"); }
  map(_mapping: Mappable): Step { return this; }
  toJSON(): unknown { return { stepType: "hbsSessionV1", entry: this.entry, direction: this.direction }; }
  static override fromJSON(_schema: unknown, value: { entry: HbsHistoryEntry<NoteDocumentV1>; direction: "before" | "after" }): Step {
    return new SessionStep(value.entry, value.direction);
  }
}
Step.jsonID("hbsSessionV1", SessionStep);
export function appendSessionHistory(transaction: Transaction, entry: HbsHistoryEntry<NoteDocumentV1>): void {
  transaction.step(new SessionStep(entry));
}
type Publication = {readonly origin: Transaction} & (
  {readonly kind: "input"; readonly entry: HbsHistoryEntry<NoteDocumentV1>; readonly before: Node; readonly intent?: BlockTextIntent}
  | {readonly kind: "history"; readonly replays: readonly SessionStep[]});
export function noteSessionHistory(session: HbsSession<NoteDocumentV1>, canReplayHistory: () => boolean = () => true): Plugin {
  const key = new PluginKey<Publication | undefined>("hbs-session-history");
  function publish(publication: Publication, document: Node): void {
    const trusted = session.getSnapshot().content;
    if (publication.kind === "input") {
      const content = noteReviewDocumentFromState(trusted.pageId, document, trusted);
      const baseline = publication.intent ? projectNoteBlockTextIntent(publication.before, session.getSnapshot().baselineProjection, publication.intent) : undefined;
      const committed = session.perform(() => session.edit(content, baseline));
      publication.entry.after = committed.after;
      publication.entry.committedBefore = committed.committedBefore;
      return;
    }
    const {replays} = publication, replay = replays.at(-1)!;
    const native = noteReviewDocumentFromState(trusted.pageId, document, trusted);
    // A native group may contain several input/split/review entries. Every
    // baseline and review delta follows PM's replay order, with the complete
    // committed native document supplied only to the final entry.
    const mappedAway = session.adapter.equal(native, trusted)
      && replays.some(step => !session.adapter.equal(step.entry.before.content, step.entry.after.content));
    let restored = trusted;
    for (const step of replays) restored = session.adapter.merge(
      step.entry[step.direction === "before" ? "after" : "before"].content,
      step.entry[step.direction].content, restored).value;
    // Native defaults on deleted/restored blocks are not new edits. Rebuild
    // their domain representation from the actual history deltas first.
    const content = mappedAway ? native : noteReviewDocumentFromState(trusted.pageId, document, restored);
    for (const step of replays) session.restore(step.entry, step.direction,
      step === replay && !mappedAway ? content : undefined);
  }
  return new Plugin<Publication | undefined>({ key,
    state: {init: () => undefined, apply: (transaction, previous) => (transaction.getMeta(key) as Publication | undefined) ?? previous},
    view(view) {
      // Reconfiguring another plugin recreates view hooks for this same state.
      // Its last publication was already committed; never publish it twice.
      let previous = key.getState(view.state);
      const commit = () => {
        const current = key.getState(view.state);
        if (!current || current === previous) return;
        previous = current;
        publish(current, view.state.doc);
      };
      return {update: commit};
    },
    filterTransaction(transaction, state) {
      if (isHistoryTransaction(transaction)) return canReplayHistory();
      const intent = transaction.getMeta(BLOCK_TEXT_INTENT) as BlockTextIntent | undefined;
      return !intent || projectNoteBlockTextIntent(state.doc, session.getSnapshot().baselineProjection, intent) !== undefined;
    },
    appendTransaction(transactions, before, after) {
      // Later normalizer passes receive only their unseen transactions. Keep
      // one publication for the complete root batch, whose final doc is read
      // after the view commits every appended pass.
      const first = transactions[0];
      const origin = first?.getMeta("appendedTransaction") as Transaction | undefined ?? first;
      if (!origin || key.getState(after)?.origin === origin) return null;
      const replays = isHistoryTransaction(origin) ? origin.steps.filter(step => step instanceof SessionStep) : [];
      if (replays.length) return after.tr.setMeta(key, {kind: "history", origin, replays} satisfies Publication);
      if (!transactions.some(transaction => transaction.docChanged && transaction.getMeta("addToHistory") !== false)
        || origin.steps.some(step => step instanceof SessionStep)) return null;
      const intent = transactions.map(transaction => transaction.getMeta(BLOCK_TEXT_INTENT) as BlockTextIntent | undefined).find(Boolean);
      const state = session.getSnapshot();
      const point = {content: state.content, baseline: state.baselineProjection, ancestor: state.ancestor,
        reviewAncestor: state.reviewAncestor, baselineAncestor: state.baselineAncestor};
      const entry: HbsHistoryEntry<NoteDocumentV1> = {before: point, after: point, committedBefore: state.observed};
      // Input rules and command probes may apply transactions to a disposable
      // EditorState before dispatch. Stage history metadata purely; only the
      // actual view commit may publish H/B′, checkpoint S′ or issue requests.
      const transaction = after.tr; appendSessionHistory(transaction, entry);
      return transaction.setMeta(key, {kind: "input", origin, entry, before: before.doc, ...(intent ? {intent} : {})} satisfies Publication);
    },
  });
}
