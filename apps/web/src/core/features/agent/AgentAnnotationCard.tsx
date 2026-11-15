import { CornerDownLeft, Pencil, Trash2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { AgentEvidenceImage, AgentEvidenceInput } from "@research-assistant/core/agent";
import { ControlHint } from "../../components/ControlHint";
import { notify } from "../../services/notifications";
import { SourceHintContent, useSourceLinkInfo, type SourceLinkInfo } from "./SourceHintContent";
import { annotationCommentPreviewParts, isTextFileAnnotation } from "./annotationPresentation";
import { AnnotationCommentPopover } from "./AnnotationCommentPopover";
import { navigateSource, type SourceNavigationLocation } from "./sourceNavigation";
import { readEvidenceImage } from "./evidenceImageGateway";

export function AgentAnnotationCard({item, disabled, number, sourceInfo, selected = false, onSelect, onUpdate, onRemove}: {
  readonly item: AgentEvidenceInput; readonly disabled: boolean; readonly selected?: boolean;
  readonly number?: number; readonly onSelect?: () => void;
  readonly sourceInfo?: SourceLinkInfo;
  readonly onUpdate?: (comment: string) => void; readonly onRemove?: () => void;
}) {
  const fallbackInfo = useSourceLinkInfo(sourceInfo ? undefined : item.source, item.title);
  const {object, lines} = sourceInfo ?? fallbackInfo;
  const [location, setLocation] = useState<SourceNavigationLocation>();
  const [locating, setLocating] = useState(false);
  const card = useRef<HTMLElement>(null);
  const mounted = useRef(true), owner = useRef({item, disabled, onUpdate, onRemove});
  owner.current = {item, disabled, onUpdate, onRemove};
  useEffect(() => {mounted.current = true; return () => {mounted.current = false;};}, []);
  useEffect(() => {setLocation(undefined);}, [item.source, item.text, item.comment]);
  function select() {
    if (!item.source) return;
    onSelect?.();
    void navigateSource(item.source, {annotation: item}).catch(reason => notify.error(reason instanceof Error ? reason.message : "无法定位来源"));
  }
  async function edit() {
    if (locating || disabled) return;
    if (!item.source) {
      setLocation({getAnchorRect: () => card.current?.isConnected ? card.current.getBoundingClientRect() : null});
      return;
    }
    onSelect?.(); setLocating(true);
    const snapshot = item;
    try {
      const target = await navigateSource(item.source, {locate: true, annotation: item});
      if (!mounted.current) return;
      if (owner.current.item.source !== snapshot.source || owner.current.item.text !== snapshot.text || owner.current.item.comment !== snapshot.comment) throw new Error("批注已更新，请重新编辑");
      if (!target) throw new Error("来源没有可显示的批注位置");
      setLocation(target);
    } catch (reason) {if (mounted.current) notify.error(reason instanceof Error ? reason.message : "无法定位批注");}
    finally {if (mounted.current) setLocating(false);}
  }
  const quote = item.displayText ?? item.text, selectedText = !item.source || isTextFileAnnotation(item, object);
  const commentPreview = annotationCommentPreviewParts(item.comment).map((part, index) => part.kind === "lineBreak"
    ? <CornerDownLeft key={index} className="agent-evidence-line-break" strokeWidth={1.75} role="img" aria-label="换行"/>
    : part.text);
  const body = <article ref={card} className={`agent-evidence-card${selected ? " is-selected" : ""}`} role="group" aria-label={`${item.source ? "定位批注" : "批注"} ${item.title}`} aria-current={selected && item.source ? "location" : undefined} tabIndex={item.source ? 0 : undefined} data-unlocated={item.source ? undefined : "true"}
        onClick={event => {if (!(event.target as Element).closest("button,a,input,textarea")) select();}}
        onKeyDown={event => {if (event.target === event.currentTarget && (event.key === "Enter" || event.key === " ")) {event.preventDefault(); select();}}}>
        <header><div className="agent-evidence-target">{selectedText ? <>
          <span className="agent-evidence-number">{number ?? 1}.</span><span className="agent-evidence-selected-label">Selected text:</span>
        </> : <>{item.image && <EvidenceImage image={item.image} alt={item.title}/>}
          <span className="agent-evidence-target-kind">{item.image ? "图片" : "文本"}</span><strong>{item.image ? item.title : quote || item.title}</strong>
        </>}</div><div className="agent-evidence-card-actions">
          {onUpdate && <ControlHint tooltipClassName="agent-evidence-hint" content="编辑批注"><button type="button" className="agent-evidence-icon-action" aria-label={`编辑批注 ${item.title}`} disabled={disabled || locating} onClick={() => void edit()}><Pencil aria-hidden="true"/></button></ControlHint>}
          {onRemove && <ControlHint tooltipClassName="agent-evidence-hint" content="删除批注"><button type="button" className="agent-evidence-icon-action" aria-label={`移除批注 ${item.title}`} disabled={disabled || locating} onClick={onRemove}><Trash2 aria-hidden="true"/></button></ControlHint>}
        </div></header>
        {selectedText && quote && <blockquote className="agent-evidence-selected-text">{quote}</blockquote>}
        {item.comment.trim() && (selectedText ? <div className="agent-evidence-text-comment">
          <span className="agent-evidence-user-comment-label">User comment:</span><p className="agent-evidence-comment">{commentPreview}</p>
        </div> : <p className="agent-evidence-comment">{commentPreview}</p>)}
      </article>;
  return <>
    {item.source ? <ControlHint preferSidePlacement tooltipClassName="agent-evidence-hint" content={<SourceHintContent path={item.source} lines={lines} includeDeveloperPath={false}/>}>{body}</ControlHint> : body}
    {location && onUpdate && <AnnotationCommentPopover location={location} label={item.source ? "编辑来源批注" : "编辑批注"} title={item.title} initialComment={item.comment} disabled={disabled}
      onCancel={() => setLocation(undefined)} onSave={comment => {if (owner.current.disabled) throw new Error("当前草稿不可编辑"); owner.current.onUpdate?.(comment); setLocation(undefined);}}
      {...(onRemove ? {onRemove: () => {owner.current.onRemove?.(); setLocation(undefined);}} : {})}/>}
  </>;
}

function EvidenceImage({image, alt}: {readonly image: AgentEvidenceImage; readonly alt: string}) {
  const [url, setUrl] = useState<string>();
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let active = true, next: string | undefined;
    setUrl(undefined); setFailed(false);
    void readEvidenceImage(image).then(bytes => {
      if (!active) return;
      next = URL.createObjectURL(new Blob([bytes.buffer as ArrayBuffer], {type: image.mediaType})); setUrl(next);
    }).catch(() => {if (active) setFailed(true);});
    return () => {active = false; if (next) URL.revokeObjectURL(next);};
  }, [image]);
  return url ? <img className="agent-evidence-image" src={url} alt={alt}/> : failed ? <span role="alert">图片快照不可用</span> : null;
}
