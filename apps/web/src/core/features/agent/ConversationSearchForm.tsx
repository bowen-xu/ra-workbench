import { useNotificationState } from "../../services/useNotificationState";
import { ControlHint } from "../../components/ControlHint";
import { Search, X } from "lucide-react";
import { useRef } from "react";

export function ConversationSearchForm({ value, onChange, onSearch, label = "搜索当前稿件的对话", showSubmit = true, autoFocus = false, onClose }: {
  readonly value: string; readonly onChange: (value: string) => void; readonly onSearch: (term: string) => void; readonly label?: string;
  readonly showSubmit?: boolean; readonly autoFocus?: boolean; readonly onClose?: () => void;
}): React.JSX.Element {
  const input = useRef<HTMLInputElement>(null);
  const composing = useRef(false);
  const [error, setError] = useNotificationState("error");
  function clear(): void { onChange(""); setError(undefined); onSearch(""); input.current?.focus(); }
  return <form className="conversation-search" role="search" aria-label={label} onSubmit={(event) => {
    event.preventDefault();
    if (composing.current) return;
    const term = value.trim();
    if ([...term].length > 200 || term.includes("\0")) { setError("搜索词最多 200 个字符，不能含空字符。"); return; }
    setError(undefined); onSearch(term);
  }}>
    <div className="conversation-search-controls">
      <input ref={input} autoFocus={autoFocus} type="search" aria-label="搜索对话标题或消息" aria-invalid={!!error} placeholder="搜索标题或消息…" value={value}
        onCompositionStart={() => { composing.current = true; }} onCompositionEnd={() => { composing.current = false; }}
        onChange={(event) => { onChange(event.target.value); setError(undefined); if (!event.target.value && !composing.current) onSearch(""); }}
        onKeyDown={(event) => {
          if (event.nativeEvent.isComposing || composing.current || event.nativeEvent.keyCode === 229) {
            if (event.key === "Enter") event.preventDefault();
            return;
          }
          if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); clear(); onClose?.(); }
        }} />
      {showSubmit && <ControlHint content="搜索对话（Enter）"><button type="submit" aria-label="搜索对话" ><Search size={15} aria-hidden="true" /></button></ControlHint>}
      {value && <ControlHint content="清空搜索（Esc）"><button type="button" aria-label="清空对话搜索"  onClick={clear}><X size={15} aria-hidden="true" /></button></ControlHint>}
    </div>

  </form>;
}
