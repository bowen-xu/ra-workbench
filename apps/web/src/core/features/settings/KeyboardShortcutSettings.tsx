import { ControlHint } from "../../components/ControlHint";
import { useRef, useState } from "react";
import { Pencil, RotateCcw, Trash2 } from "lucide-react";
import { bindingFromEvent, defaultShortcuts, formatShortcut, sameShortcut, shortcutActions, type ShortcutAction } from "./keyboardShortcuts";
import { useShortcutPreferences } from "./ShortcutPreferences";
import "./KeyboardShortcutSettings.css";

export function KeyboardShortcutSettings(): React.JSX.Element {
  const { bindings, update, reset } = useShortcutPreferences();
  const [recording, setRecording] = useState<ShortcutAction>();
  const [error, setError] = useState<string>();
  const defaults = defaultShortcuts();
  const changed = shortcutActions.some(action => !sameShortcut(bindings[action.id], defaults[action.id]));
  const recordButtons = useRef<Record<ShortcutAction, HTMLButtonElement | null>>({ toggleChat: null, toggleConversations: null });
  return <section className="settings-card keyboard-shortcut-settings" aria-labelledby="keyboard-shortcut-heading">
    <h2 id="keyboard-shortcut-heading">快捷键</h2>
    <p>点击快捷键后按下新的组合。修改立即生效，并保存在本机；按 Esc 取消录入。</p>
    <div className="keyboard-shortcut-rows">
      {shortcutActions.map(action => <div className="keyboard-shortcut-row" key={action.id}>
        <div className="setting-copy"><span className="setting-title">{action.label}</span><p>{action.id === "toggleChat" ? "在当前工作区显示或收起聊天。" : "显示或收起聊天中的对话列表。"}</p></div>
        <div className="keyboard-shortcut-controls">
        <button ref={node => { recordButtons.current[action.id] = node; }} type="button" className="keyboard-shortcut-record" aria-label={`修改${action.label}快捷键`}
          aria-pressed={recording === action.id} data-shortcut-recorder={recording === action.id ? "true" : undefined} data-tab-action={recording === action.id ? "both" : undefined}
          onClick={() => { setRecording(action.id); setError(undefined); }}
          onBlur={() => { if (recording === action.id) setRecording(undefined); }}
          onKeyDown={event => {
            if (recording !== action.id || event.nativeEvent.isComposing || event.keyCode === 229 || event.repeat) return;
            if (event.key === "Tab") { event.preventDefault(); event.stopPropagation(); setRecording(undefined); return; }
            event.preventDefault(); event.stopPropagation();
            if (event.key === "Escape") { setRecording(undefined); setError(undefined); return; }
            const binding = bindingFromEvent(event);
            if (!binding) return;
            const problem = update(action.id, binding);
            setError(problem);
            if (!problem) setRecording(undefined);
          }}>
          {recording === action.id ? "请按快捷键…" : <><kbd>{bindings[action.id] === null ? "未设置" : formatShortcut(bindings[action.id])}</kbd><Pencil size={16} aria-hidden="true" /></>}
        </button>
        {recording === action.id && <button type="button" className="keyboard-shortcut-cancel" onMouseDown={event => event.preventDefault()} onClick={() => { setRecording(undefined); setError(undefined); }}>取消</button>}
        <div className="keyboard-shortcut-actions">
        {bindings[action.id] !== null && <ControlHint content="删除快捷键"><button className="keyboard-shortcut-delete" type="button" aria-label={`删除${action.label}快捷键`} onClick={() => { update(action.id, null); setRecording(undefined); setError(undefined); recordButtons.current[action.id]?.focus(); }}><Trash2 size={17} aria-hidden="true" /></button></ControlHint>}
        {!sameShortcut(bindings[action.id], defaults[action.id]) && <ControlHint content="恢复默认快捷键"><button type="button" className="keyboard-shortcut-restore" aria-label={`恢复${action.label}默认快捷键`}
          onClick={() => { setError(update(action.id, defaults[action.id])); setRecording(undefined); recordButtons.current[action.id]?.focus(); }}><RotateCcw size={17} aria-hidden="true" /></button></ControlHint>}
        </div></div>
      </div>)}
    </div>
    <p className="keyboard-shortcut-feedback" role="status">{error ?? (recording ? "正在录入：按组合键保存，按 Esc 取消。" : "仅在应用窗口内生效，输入法组合输入和模态弹窗期间暂停。")}</p>
    {changed && <button type="button" className="keyboard-shortcut-reset" onClick={() => { reset(); setRecording(undefined); setError(undefined); recordButtons.current.toggleChat?.focus(); }}>恢复默认快捷键</button>}
  </section>;
}
