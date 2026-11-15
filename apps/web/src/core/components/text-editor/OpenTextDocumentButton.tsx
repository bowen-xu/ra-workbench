import { ExternalLink } from "lucide-react";
import { openTextDocument, type TextDocumentRequest } from "./textDocumentEvents";

export function OpenTextDocumentButton({ document, disabled, onOpened }: {
  readonly document: TextDocumentRequest;
  readonly disabled?: boolean;
  readonly onOpened?: () => void;
}): React.JSX.Element {
  return <button type="button" disabled={disabled} onClick={() => { if (openTextDocument(document)) onOpened?.(); }}><ExternalLink size={15} aria-hidden="true" />在标签页中打开</button>;
}
