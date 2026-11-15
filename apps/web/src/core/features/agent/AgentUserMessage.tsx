import { InlineReference } from "../../components/InlineReference";
import { Box, ChevronDown, ChevronUp, FileText, Plug } from "lucide-react";
import { Fragment, useId, useLayoutEffect, useRef, useState } from "react";
import { captureTurnDisclosureAnchor } from "./presentation/turnDisclosureAnchor";
import type { AgentReferenceInput } from "@research-assistant/core/agent";
import type { WritingAgentMessage } from "./presentation/WritingAgentMessage";
import type { ReadMessageData } from "./AgentMessageData";
import type { AgentResourceGateways } from "./agentResourceGateways";
import { AgentReferenceDocument } from "./AgentReferenceDocument";
import { useAgentUserMessageDraft } from "./AgentUserMessageScope";
import { useUserMessageSelectionCopy } from "./useUserMessageSelectionCopy";
import { ControlHint } from "../../components/ControlHint";
import "./AgentComposerTextInput.css";
import "./AgentUserMessage.css";
import { AgentEvidenceReferences } from "./AgentEvidenceReferences";
import { useVisibleLayoutMeasurement } from "../../components/layout/useVisibleLayoutMeasurement";

/** Read-only references share the message's frozen draft with its text and copy action. */
export function AgentUserMessageEvidence({ message, read }: {
  readonly message: WritingAgentMessage;
  readonly read?: ReadMessageData | undefined;
}): React.JSX.Element | null {
  const draft = useAgentUserMessageDraft(message, read);
  if (!draft.evidence?.length) return null;
  return <div className="agent-user-message-evidence"><AgentEvidenceReferences evidence={draft.evidence} /></div>;
}

/** Recover only durable bindings, never recognize references by an @name regex. */
export function AgentUserMessage({ message, read, gateways, reveal = false }: {
  readonly message: WritingAgentMessage;
  readonly read?: ReadMessageData | undefined;
  readonly gateways?: AgentResourceGateways | undefined;
  readonly reveal?: boolean;
}): React.JSX.Element {
  const draft = useAgentUserMessageDraft(message, read);
  const [selected, setSelected] = useState<AgentReferenceInput | null>(null);
  const text = useRef<HTMLParagraphElement>(null);
  const textId = useId();
  const anchor = useRef<ReturnType<typeof captureTurnDisclosureAnchor>>(undefined);
  const [overflow, setOverflow] = useState(false), [expandedId, setExpandedId] = useState<string | null>(null);
  const expanded = expandedId === message.id;
  useLayoutEffect(() => { if (reveal) setExpandedId(message.id); }, [reveal, message.id]);
  useLayoutEffect(() => { anchor.current?.restore(); }, [expanded]);
  useLayoutEffect(() => () => anchor.current?.release(), []);
  useUserMessageSelectionCopy(text, draft);
  useVisibleLayoutMeasurement(() => text.current, element => {
    const lineHeight = Number.parseFloat(getComputedStyle(element).lineHeight);
    setOverflow(Number.isFinite(lineHeight) && element.scrollHeight > Math.ceil(lineHeight * 8) + 1);
  });
  function reference(input: AgentReferenceInput, key: number) {
    const kind = input.type === "skill" ? "技能" : input.target.kind === "tool" ? "工具" : "文件";
    const Icon = input.type === "skill" ? Box : input.target.kind === "tool" ? Plug : FileText;
    const source = input.type === "skill" ? input.identity.source_id : input.target.kind === "tool" ? input.target.serverId : input.target.resourceId;
    return <ControlHint key={key} content={`${input.name} · ${source}`}><InlineReference as="button" type="button" className="agent-composer-reference agent-message-reference" label={input.name}
      icon={<Icon aria-hidden="true" />} aria-label={`${kind} ${input.name}`} onClick={() => setSelected(input)} /></ControlHint>;
  }
  let end = 0;
  const segments = draft.bindings.map((binding, index) => {
    const start = end, prefix = draft.text.slice(start, binding.startUtf16); end = binding.endUtf16;
    return <Fragment key={index}><span data-agent-copy-start={start} data-agent-copy-end={binding.startUtf16}>{prefix}</span>
      <span data-agent-copy-start={binding.startUtf16} data-agent-copy-end={binding.endUtf16} data-agent-copy-reference="true">{reference(binding.input, index)}</span></Fragment>;
  });
  return <><div className="agent-user-message-text" data-collapsed={!expanded}>
    <p ref={text} id={textId}>{segments}<span data-agent-copy-start={end} data-agent-copy-end={draft.text.length}>{draft.text.slice(end)}</span>{draft.unboundReferences?.map((input, index) => <Fragment key={`attachment:${index}`}> {reference(input, index)}</Fragment>)}</p>
  </div>{overflow && <div className="agent-user-message-disclosure">
    {!expanded && <span className="agent-user-message-ellipsis" aria-hidden="true">...</span>}
    <ControlHint content={expanded ? "收起" : "展开"}><button type="button" aria-label={expanded ? "收起" : "展开"} aria-controls={textId} aria-expanded={expanded} onClick={event => {
      const target = expanded ? event.currentTarget : text.current;
      if (target) anchor.current = captureTurnDisclosureAnchor(target);
      setExpandedId(expanded ? null : message.id);
    }}>{expanded ? <ChevronUp size={14} aria-hidden="true" /> : <ChevronDown size={14} aria-hidden="true" />}</button></ControlHint>
  </div>}
    {selected && <AgentReferenceDocument reference={selected} onClose={() => setSelected(null)} {...(gateways ? { gateways } : {})} />}
  </>;
}
