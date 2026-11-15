import type {ReactNode} from "react";
import "./AgentSubmissionFeedback.css";

/** Production feedback shell; hosts supply recovery actions, never their own typography. */
export function AgentSubmissionFeedback({children, label = "发送恢复", className = ""}: {
  readonly children: ReactNode;
  readonly label?: string;
  readonly className?: string;
}): React.JSX.Element {
  return <section aria-label={label} className={`agent-submission-feedback ${className}`.trim()}>{children}</section>;
}
