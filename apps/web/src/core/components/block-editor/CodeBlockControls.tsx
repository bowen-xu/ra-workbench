import { Check, ChevronDown, Copy } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { ActionMenu, ActionMenuItem } from "../ActionMenu";
import { ControlHint } from "../ControlHint";
import { notify } from "../../services/notifications";
import { useEditorReadOnly } from "./EditorReadOnly";
import { blockCodeLanguages } from "./codeBlockSyntax";

export function CodeBlockView({ language, choose, source, contentRef }: {
  readonly language: string;
  readonly choose: (language: string) => void;
  readonly source: () => string;
  readonly contentRef: (element: HTMLElement | null) => void;
}): React.JSX.Element {
  const readOnly = useEditorReadOnly();
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">("idle");
  const active = useRef(true);
  useEffect(() => { active.current = true; return () => { active.current = false; }; }, []);
  const name = blockCodeLanguages[language as keyof typeof blockCodeLanguages]?.name ?? language;
  const copy = async () => {
    try { await navigator.clipboard.writeText(source()); if (active.current) setCopyState("copied"); }
    catch { if (active.current) { setCopyState("failed"); notify.error("复制失败，请重试或手动选择代码复制"); } }
  };
  return <>
    <div className="code-block-controls" contentEditable={false}>
      <ActionMenu label="代码语言" menuLabel="选择代码语言" className="code-language-control"
        triggerClassName="code-language-trigger" contentClassName="code-language-menu" portal disabled={readOnly}
        trigger={<><span>{name}</span><ChevronDown aria-hidden="true" /></>}>
        {Object.entries(blockCodeLanguages).map(([key, item]) => <ActionMenuItem key={key}
          role="menuitemradio" aria-checked={language === key} onClick={() => { if (!readOnly) choose(key); }}>
          {item.name}{language === key && <Check aria-hidden="true" />}
        </ActionMenuItem>)}
      </ActionMenu>
      <ControlHint content={copyState === "copied" ? "已复制" : copyState === "failed" ? "复制失败，请重试" : "复制代码"}>
        <button type="button" className="code-copy-button" aria-label="复制代码"
          onMouseDown={event => event.preventDefault()} onMouseLeave={() => setCopyState("idle")} onClick={() => { void copy(); }}>
          {copyState === "copied" ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />}
        </button>
      </ControlHint>
    </div>
    <pre><code ref={contentRef} /></pre>
  </>;
}
