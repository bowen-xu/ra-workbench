import { Popover } from "../../components/overlay/OverlayPopup";
import { MessageSquareQuote, X } from "lucide-react";
import { PopoverDisclosure, usePopoverStore } from "@ariakit/react";
import type { AgentEvidenceInput } from "@research-assistant/core/agent";
import { useState } from "react";
import "./AgentEvidenceReferences.css";
import { AgentAnnotationCard } from "./AgentAnnotationCard";
import { useSourceLinkInfos } from "./SourceHintContent";
import { isTextFileAnnotation } from "./annotationPresentation";

export function AgentEvidenceReferences({ evidence = [], onChange, onClear, disabled = false }: {
  readonly evidence?: readonly AgentEvidenceInput[];
  readonly onChange?: (evidence: readonly AgentEvidenceInput[]) => void;
  readonly disabled?: boolean;
  readonly onClear?: () => void;
}): React.JSX.Element | null {
  if (!evidence.length) return null;
  return <div className="agent-evidence-references"><EvidenceGroup evidence={evidence} disabled={disabled} {...(onChange ? {onChange} : {})} {...(onClear ? {onClear} : {})}/></div>;
}
function EvidenceGroup({evidence, onChange, onClear, disabled}: {
  readonly evidence: readonly AgentEvidenceInput[];
  readonly onChange?: (evidence: readonly AgentEvidenceInput[]) => void; readonly disabled: boolean;
  readonly onClear?: () => void;
}) {
  const popover = usePopoverStore({placement:"top-start"});
  const [selectedIndex, setSelectedIndex] = useState<number>();
  const sources = useSourceLinkInfos(evidence.flatMap(item => item.source ? [item.source] : []));
  let selectedTextNumber = 0;
  const entries = evidence.map((item,index)=>({item,index, sourceInfo: item.source ? sources.get(item.source) ?? {lines: [item.title, "正在读取来源…"]} : {lines: [item.title]},
    number: !item.source || isTextFileAnnotation(item, sources.get(item.source)?.object) ? ++selectedTextNumber : undefined}));
  const label = "批注";
  return <div className="agent-evidence-group">
      <PopoverDisclosure store={popover} className="agent-evidence-disclosure"><MessageSquareQuote aria-hidden="true"/><span>{entries.length} 条{label}</span></PopoverDisclosure>
      {onChange && <button type="button" className="agent-evidence-clear" aria-label="清除全部批注" disabled={disabled} onClick={() => {onChange([]); popover.hide(); onClear?.();}}><X aria-hidden="true"/></button>}
      <Popover store={popover} portal gutter={8} autoFocusOnShow={false} className="agent-evidence-popover" aria-label={`${label}引用内容`}>
        <div className="agent-evidence-list">
        {entries.map(({item, index, number, sourceInfo}) => <AgentAnnotationCard key={index} item={item} sourceInfo={sourceInfo} disabled={disabled} selected={selectedIndex === index} onSelect={() => setSelectedIndex(index)} {...(number ? {number} : {})} {...(onChange ? {
          onRemove: () => {setSelectedIndex(undefined); onChange(evidence.filter((_, position) => position !== index));},
          onUpdate: (comment: string) => onChange(evidence.map((entry, position) => position === index ? {...entry, comment} : entry)),
        } : {})}/>)}
        </div>
      </Popover>
    </div>;
}
