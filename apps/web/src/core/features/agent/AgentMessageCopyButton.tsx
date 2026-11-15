import { ControlHint } from "../../components/ControlHint";
import { Check, Copy } from "lucide-react";
import { useState } from "react";
import { copyText } from "./copyText";
import { copyComposerClipboard } from "./agentComposerClipboard";
import { useUserMessageCopyDraft } from "./AgentUserMessageScope";

interface Props {
  readonly value: string;
  readonly className?: string;
  readonly messageId?: string | undefined;
  readonly label?: string;
}

/** Copies one complete user/assistant message; tool groups use their own detail scope. */
export function AgentMessageCopyButton({ value, className = "agent-message-copy", messageId, label = "复制消息" }: Props): React.JSX.Element {
  const [copied, setCopied] = useState(false);
  const draft = useUserMessageCopyDraft(messageId);
  async function copy(): Promise<void> {
    if (await (draft?.bindings.length ? copyComposerClipboard({text: draft.text, bindings: draft.bindings}) : copyText(value))) {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1400);
    }
  }
  return <ControlHint content={copied ? "已复制" : label}><button aria-label={copied ? "已复制消息" : label} className={className} onClick={() => void copy()}  type="button">
    {copied ? <Check aria-hidden="true" size={13} /> : <Copy aria-hidden="true" size={13} />}
  </button></ControlHint>;
}
