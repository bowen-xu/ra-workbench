import "./BlockCaption.css";

/** Static/export presentation. Editable captions use captionBlockSpec in the
 * owning editor's document, without a second EditorView. */
export function BlockCaption({value}: {readonly value: string; readonly label?: string}): React.JSX.Element | null {
  return value ? <div className="block-caption"><span>{value}</span></div> : null;
}
