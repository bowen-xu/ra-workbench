import type { ReactNode } from "react";
import { AgentFailureMessage } from "./AgentFailureMessage";

/** Recovery feedback uses the same visual and diagnostic affordances as chat errors. */
export function AgentRecoveryMessage({ id, label, content, diagnosticText, retry, children, embedded = false }: {
  readonly id: string; readonly label: string; readonly content: string; readonly diagnosticText?: string | undefined;
  readonly retry?: {readonly label: string; readonly onClick: () => void} | undefined; readonly children?: ReactNode;
  readonly embedded?: boolean;
}): React.JSX.Element {
  const body = <><AgentFailureMessage message={{id, role: "assistant", content, failure: true, ...(diagnosticText ? {failureDiagnosticText: diagnosticText} : {})}}
    {...(retry ? {retry} : {})} />{children}</>;
  if (embedded) return body;
  return <article className="writing-agent-message assistant failure" role="alert" aria-label={label}>
    <div className="writing-agent-message-bubble" data-selectable-text>
      {body}
    </div>
  </article>;
}
