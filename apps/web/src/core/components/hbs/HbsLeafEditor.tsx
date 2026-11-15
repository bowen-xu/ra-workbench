import { useSyncExternalStore } from "react";
import type { DocumentPath, HbsSession } from "@research-assistant/core/hbs";
import { TextEditor, type TextEditorProps } from "../text-editor/TextEditor";
import type { EditorReviewConfiguration } from "../editor-diff/EditorReviewActions";

/** CM owns input and action history. The same session owns the complete typed
 * document; field editors never save detached strings to a separate store. */
export function HbsLeafEditor<D>({ session, path, label, configuration, historyActions, stateRetention, readOnly = false, readOnlyHistory=false, presentation }: {
  readonly session: HbsSession<D>; readonly path: DocumentPath; readonly label: string; readonly configuration?: EditorReviewConfiguration;
  readonly historyActions?: TextEditorProps["historyActions"];
  readonly stateRetention?: TextEditorProps["stateRetention"];
  readonly readOnly?: boolean;
  readonly readOnlyHistory?: boolean;
  readonly presentation?: TextEditorProps["presentation"];
}): React.JSX.Element {
  const state = useSyncExternalStore(session.subscribe, session.getSnapshot), adapter = session.adapter;
  const read = (value: D) => String(adapter.read(value, path) ?? "");
  // A read-only history host records complete document actions. Replaying it
  // must not manufacture a missing leaf (for example kind on a null resource).
  const write = (value: D, text: string) => readOnlyHistory ? value : adapter.replace(value, path, text);
  const value = read(state.content), baseline = read(state.baselineProjection);
  const conflicts = state.conflicts.filter(change => JSON.stringify(change.path) === JSON.stringify(path) && typeof change.incoming === "string");
  return <TextEditor documentId={JSON.stringify(path)} path={label} value={value} baseline={baseline}
    {...(presentation ? {presentation} : {})}
    readOnly={readOnly}
    readOnlyHistory={readOnlyHistory}
    {...(historyActions ? {historyActions} : {})}
    {...(stateRetention ? {stateRetention} : {})}
    dirty={!adapter.equal(state.content, state.observed.saved)} {...(configuration ? { reviewConfiguration: configuration } : {})}
    onChange={(text, original) => { const current = session.getSnapshot(); session.edit(write(current.content, text), write(current.baselineProjection, original)); }}
    onSave={() => { void session.save(); }}
    onReview={(text, original) => { const current = session.getSnapshot(); session.review(write(current.content, text), write(current.baselineProjection, original)); }}
    incoming={conflicts.map(change => ({ id: change.id, ...(adapter.textIncoming?.(change,state.content,state.baselineProjection,state.observed.saved)
      ?? {from:0,to:value.length,text:change.incoming as string,base:String(change.before??"")}) }))}
    onIncomingResolve={resolution => { const current=session.getSnapshot();session.resolveMany(resolution.ids,resolution.action,
      {content:write(current.content,resolution.value),baseline:write(current.baselineProjection,resolution.baseline)}); }}
    history={{ perform: action => {
      const entry = session.perform(action);
      const shown = session.getSnapshot();
      return { before: { value: read(entry.before.content), baseline: read(entry.before.baseline),
        restore: text => session.restore(entry, "before", write(adapter.merge(entry.after.content,entry.before.content,session.getSnapshot().content).value, text)) },
      after: { value: read(shown.content), baseline: read(shown.baselineProjection),
        restore: text => session.restore(entry, "after", write(adapter.merge(entry.before.content,entry.after.content,session.getSnapshot().content).value, text)) } };
    }, restoreEdit: (text, changes) => {
      const current = session.getSnapshot(); let baseline = current.baselineProjection;
      for (const change of changes) baseline = adapter.merge(write(baseline, change.before), write(baseline, change.after), baseline).value;
      session.edit(write(current.content, text), baseline);
    } }} />;
}
