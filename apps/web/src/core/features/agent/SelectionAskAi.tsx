import { forwardRef, type ComponentPropsWithoutRef } from "react";
import { MessageSquareQuote } from "lucide-react";
import { ControlHint } from "../../components/ControlHint";
import { AnnotationCommentEditor } from "./AnnotationCommentEditor";
import type { SelectionAnnotationIntent } from "./submitEvidenceAnnotation";

/** Hosts supply placement and evidence capture; the control and editor are shared. */
export const SelectionAskAiButton = forwardRef<HTMLButtonElement,
  Omit<ComponentPropsWithoutRef<"button">, "children" | "type" | "aria-label">
>(function SelectionAskAiButton(props, ref) {
  return <ControlHint content="问AI"><button {...props} ref={ref} type="button" aria-label="问AI">
    <MessageSquareQuote aria-hidden="true" size={18} />
  </button></ControlHint>;
});

export function SelectionAskAiEditor({ title, disabled = false, onCancel, onSubmit }: {
  readonly title: string;
  readonly disabled?: boolean;
  readonly onCancel: () => void;
  readonly onSubmit: (comment: string, intent?: SelectionAnnotationIntent) => Promise<void>;
}): React.JSX.Element {
  return <AnnotationCommentEditor mode="create" title={title} disabled={disabled} onCancel={onCancel}
    onSave={comment => onSubmit(comment)} onSend={comment => onSubmit(comment, "send")}
    onDismiss={comment => onSubmit(comment, "silent")} />;
}
