import { useSyncExternalStore } from "react";
import { paragraph, type HbsClient } from "@research-assistant/core/hbs";
import { TextEditor, type TextEditorProps } from "../text-editor/TextEditor";

export interface HbsTextEditorProps extends Omit<TextEditorProps,
  "documentId" | "value" | "baseline" | "dirty" | "onChange" | "onSave" | "onReview" | "incoming" | "onIncomingResolve" | "history"> {
  readonly client: HbsClient;
}
/** Production composition point. Supply a client with any diff, wire and
 * repository adapters. Notes/Writing ownership stays in the consuming host. */
export function HbsTextEditor({ client, ...presentation }: HbsTextEditorProps): React.JSX.Element {
  const { observed, draft } = useSyncExternalStore(client.subscribe, client.getSnapshot);
  const value = draft?.value ?? paragraph(observed.saved, client.documentId).text;
  const baseline = draft?.editedBaseline ?? paragraph(observed.baseline, client.documentId).text;
  const dirty=value!==paragraph(observed.saved,client.documentId).text||baseline!==paragraph(observed.baseline,client.documentId).text;
  return <TextEditor {...presentation} documentId={client.documentId} value={value} baseline={baseline} dirty={dirty}
    onChange={client.edit} onSave={client.save} onReview={client.review} incoming={client.getIncoming()}
    onIncomingResolve={client.resolveIncoming} history={client.history} />;
}
