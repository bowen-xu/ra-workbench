import { Popover } from "../../components/overlay/OverlayPopup";
import { Composite, CompositeItem, CompositeProvider, PopoverDisclosure, PopoverProvider, usePopoverStore } from "@ariakit/react";
import type { LlmModelDescriptor, LlmReasoningEffort } from "@research-assistant/core/ai";
import { Brain, Check, ChevronDown, ChevronRight, RotateCcw, Zap } from "lucide-react";
import { useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { ControlHint } from "../../components/ControlHint";

import "./AgentModelPicker.css";

interface AgentModelPickerProps {
  readonly disabled: boolean;
  readonly label: string;
  readonly modelLabel: string;
  readonly title: string;
  readonly modelId?: string | undefined;
  readonly models: readonly LlmModelDescriptor[];
  readonly efforts: readonly LlmReasoningEffort[];
  readonly effort?: LlmReasoningEffort | undefined;
  readonly isDefault: boolean;
  readonly canUseDefault: boolean;
  readonly openAi: boolean;
  readonly supportsFastMode: boolean;
  readonly fastMode: boolean;
  readonly onDefault: () => void;
  readonly onReset: () => void;
  readonly onModel: (modelId: string) => void;
  readonly onEffort: (effort: LlmReasoningEffort) => void;
  readonly onFastMode: (fastMode: boolean) => void;
}

const effortLabels: Record<LlmReasoningEffort, string> = {
  none: "None", low: "Low", medium: "Medium", high: "High", xhigh: "Extra high", max: "Max",
};

/** Shared production presentation; both native and mock gateways feed this picker. */
export function AgentModelPicker(props: AgentModelPickerProps): React.JSX.Element {
  const [open, setOpenState] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const dismissedOutside = useRef(false);
  const [editingTrigger, setEditingTrigger] = useState<{
    readonly width: number;
    readonly compact: boolean;
    readonly modelLabel: string;
    readonly effort: LlmReasoningEffort | undefined;
    readonly fastMode: boolean;
  }>();
  function setOpen(nextOpen: boolean): void {
    if (nextOpen && !open && trigger.current) {
      dismissedOutside.current = false;
      setEditingTrigger({ width: trigger.current.getBoundingClientRect().width,
        compact: trigger.current.classList.contains("is-compact"), modelLabel: props.modelLabel,
        effort: props.effort, fastMode: props.fastMode });
    } else if (!nextOpen) setEditingTrigger(undefined);
    setOpenState(nextOpen);
  }
  const [view, setView] = useState<"effort" | "models">("effort");
  const container = useRef<HTMLDivElement>(null);
  const label = useRef<HTMLSpanElement>(null);
  const [compact, setCompact] = useState(false);
  const popover = usePopoverStore({ placement: (editingTrigger?.compact ?? compact) ? "top-end" : "top", open, setOpen });
  useLayoutEffect(() => {
    if (!open) return;
    const ownerDocument = trigger.current?.ownerDocument;
    if (!ownerDocument) return;
    const dismiss = (event: PointerEvent) => {
      const { contentElement, disclosureElement } = popover.getState();
      const path = event.composedPath();
      if (!contentElement || path.includes(contentElement) || (disclosureElement && path.includes(disclosureElement))) return;
      // Resize handles capture the pointer and may prevent click/focus events.
      // Finish selection before their handlers run, without consuming the gesture.
      dismissedOutside.current = true;
      popover.hide();
    };
    ownerDocument.addEventListener("pointerdown", dismiss, true);
    return () => ownerDocument.removeEventListener("pointerdown", dismiss, true);
  }, [open, popover]);
  const [rightPadding, setRightPadding] = useState(12);
  useLayoutEffect(() => {
    if (!open) return;
    const composer = container.current?.closest(".agent-reference-anchor");
    const send = composer?.querySelector(".writing-agent-send, .paper-ai-reading-send, .note-agent-send");
    if (!composer || !send) return;
    const measure = () => setRightPadding(Math.max(12, window.innerWidth - send.getBoundingClientRect().right));
    const observer = new ResizeObserver(measure);
    observer.observe(composer); observer.observe(send);
    window.addEventListener("resize", measure); measure();
    return () => { observer.disconnect(); window.removeEventListener("resize", measure); };
  }, [open]);
  const [preferredWidth, setPreferredWidth] = useState<number>();
  useLayoutEffect(() => {
    const host = container.current, content = label.current;
    if (!host || !content) return;
    const measure = () => {
      if (open) return;
      const width = content.getBoundingClientRect().width + 20;
      if (width <= 20) return;
      setPreferredWidth(width);
      setCompact(host.getBoundingClientRect().width + .5 < width);
    };
    const observer = new ResizeObserver(measure);
    observer.observe(host); observer.observe(content); measure();
    return () => observer.disconnect();
  }, [open, props.modelLabel, props.effort, props.fastMode, props.openAi]);
  const modelButton = useRef<HTMLButtonElement>(null);
  const hasEffort = props.efforts.length > 0 && props.effort !== undefined;
  const effortIndex = Math.max(0, props.efforts.indexOf(props.effort ?? props.efforts[0]!));
  const progress = props.efforts.length > 1 ? effortIndex / (props.efforts.length - 1) * 100 : 0;
  const showingModels = view === "models" || !props.openAi;
  const selectingEffort = open && !showingModels;
  const triggerWidth = editingTrigger?.width ?? preferredWidth;
  const triggerCompact = editingTrigger?.compact ?? compact;
  const triggerEffort = editingTrigger ? editingTrigger.effort : props.effort;

  return <PopoverProvider store={popover}>
    <div ref={container} className="agent-model-picker" style={triggerWidth ? { flexBasis: triggerWidth, width: triggerWidth } : undefined}>
      <ControlHint content={props.modelId === undefined ? props.title : props.openAi ? "选择强度" : "选择模型"} wrap><PopoverDisclosure store={popover} type="button" aria-label={props.label} disabled={props.disabled}
        ref={trigger} style={editingTrigger ? { width: editingTrigger.width } : undefined}
        className={`agent-model-picker-trigger${triggerCompact ? " is-compact" : ""}${selectingEffort ? " is-selecting-effort" : ""}`} onClick={() => setView(props.openAi ? "effort" : "models")}>
        <span ref={label} className="agent-model-picker-label">
        {props.openAi && (editingTrigger?.fastMode ?? props.fastMode) && <Zap aria-hidden="true" size={13} fill="currentColor" />}
        <span className="agent-model-picker-name">{editingTrigger?.modelLabel ?? props.modelLabel}</span>
        {props.openAi && triggerEffort !== undefined && <span className="agent-model-picker-effort">{effortLabels[triggerEffort]}</span>}
        <ChevronDown aria-hidden="true" size={14} />
        </span>
        {selectingEffort && !triggerCompact && <span className="agent-model-picker-selecting-label">选择强度<ChevronDown aria-hidden="true" size={14} /></span>}
        {triggerCompact && <Brain aria-hidden="true" size={18} />}
      </PopoverDisclosure></ControlHint>
      <Popover portal fixed modal={false} unmountOnHide gutter={8} overflowPadding={{ top: 12, bottom: 12, left: 12, right: rightPadding }} fitViewport
        autoFocusOnHide={() => !dismissedOutside.current}
        aria-label={showingModels ? "选择模型" : "模型和推理强度"} className={`agent-model-picker-popover${showingModels ? " is-model-list" : ""}`}
        hideOnEscape={(event) => {
          event.stopPropagation();
          if (showingModels && props.openAi) {
            setView("effort");
            modelButton.current?.focus();
            return false;
          }
          return true;
        }}>
        {showingModels ? <>
          <div className="agent-model-picker-title">选择模型</div>
          <CompositeProvider>
            <Composite role="menu" aria-label="可用模型" className="agent-model-picker-options">
              <CompositeItem render={<button type="button" />} role="menuitem" disabled={!props.canUseDefault}
                className="agent-model-picker-option agent-model-picker-default" onClick={() => { props.onDefault(); setOpen(false); }}>
                <span>默认<small>使用此功能配置的默认模型</small></span>
              </CompositeItem>
              {props.models.map((model) => <CompositeItem key={model.id} render={<button type="button" />}
                role="menuitemradio" aria-checked={model.id === props.modelId} className="agent-model-picker-option"
                onClick={() => { props.onModel(model.id); setOpen(false); }}>
                <span>{model.displayName}</span>
                {model.id === props.modelId && <Check aria-hidden="true" size={18} />}
              </CompositeItem>)}
              {props.modelId !== undefined && !props.models.some((model) => model.id === props.modelId) &&
                <div className="agent-model-picker-unlisted">当前模型：{props.modelLabel}</div>}
            </Composite>
          </CompositeProvider>
        </> : <>
          <div className="agent-model-picker-effort-header">
            <ControlHint content={props.supportsFastMode
              ? <span className="control-hint-lines"><span>快速模式</span><span className="control-hint-caption">更多用量</span></span>
              : "当前模型不支持 Fast mode"}>
            <span className="agent-model-picker-icon-anchor">
            <button type="button" aria-label="Fast mode"
              aria-pressed={props.fastMode} disabled={!props.supportsFastMode} className="agent-model-picker-fast"
              onClick={() => props.onFastMode(!props.fastMode)}><Zap aria-hidden="true" size={16} fill={props.fastMode ? "currentColor" : "none"} /></button>
            </span>
            </ControlHint>
            <button type="button" aria-label="选择模型" ref={modelButton} className="agent-model-picker-model-link"
              onClick={() => setView("models")}>
              <span aria-live="polite" className="agent-model-picker-effort-value">{hasEffort ? effortLabels[props.effort!] : "选择模型"}</span>
              <span className="agent-model-picker-model-name">{props.modelLabel}<ChevronRight aria-hidden="true" size={15} /></span>
            </button>
            <ControlHint content="恢复默认">
            <span className="agent-model-picker-icon-anchor">
            <button type="button" aria-label="恢复默认"
              disabled={!props.canUseDefault || props.isDefault}
              className="agent-model-picker-reset" onClick={props.onReset}>
              <RotateCcw aria-hidden="true" size={16} />
            </button>
            </span>
            </ControlHint>
          </div>
          {hasEffort && <div className="agent-model-picker-slider" style={{ "--effort-progress": `${progress}%`, "--effort-position": progress / 100 } as CSSProperties}>
            <div aria-hidden="true" className="agent-model-picker-slider-fill" />
            <div aria-hidden="true" className="agent-model-picker-ticks">
              {props.efforts.map((effort) => <span key={effort} className={props.efforts.indexOf(effort) <= effortIndex ? "selected" : ""} />)}
            </div>
            <input type="range" aria-label="选择推理强度" aria-valuetext={effortLabels[props.effort!]} min={0}
              max={Math.max(0, props.efforts.length - 1)} step={1} value={effortIndex} disabled={props.efforts.length < 2}
              onInput={(event) => { const effort = props.efforts[Number(event.currentTarget.value)]; if (effort) props.onEffort(effort); }} />
            <div aria-hidden="true" className="agent-model-picker-slider-thumb" />
          </div>}
        </>}
      </Popover>
    </div>
  </PopoverProvider>;
}
