import type {AgentUserInput} from "@research-assistant/core/agent";
import { useEffect, useRef, useState } from "react";

export interface AgentSteeringRecoveryProps {
  readonly read:(turnId:string)=>Promise<{readonly inputs:readonly {readonly inputId:string;readonly text:string;readonly userInput?:readonly AgentUserInput[]}[]}>;
  readonly className?:string;
  readonly turns: readonly { readonly id: string; readonly label: string }[];
  readonly disabled: boolean;
  readonly onRestore: (text: string,userInput?:readonly AgentUserInput[]) => void;
}

/** Mounted per manuscript/conversation. Restoring only appends local draft text. */
export function AgentSteeringRecovery(props: AgentSteeringRecoveryProps): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const [selected, setSelected] = useState("");
  const [retry, setRetry] = useState(0);
  const [state, setState] = useState<{ turn: string; inputs: readonly {readonly inputId:string;readonly text:string;readonly userInput?:readonly AgentUserInput[]}[]; phase: "loading" | "ready" | "error" } | null>(null);
  const [restored, setRestored] = useState<ReadonlySet<string>>(new Set());
  const restoredNow = useRef(new Set<string>());
  const turn = props.turns.some((value) => value.id === selected) ? selected : props.turns.at(-1)?.id;
  useEffect(() => {
    if (!open || !turn || props.disabled) return;
    let current = true;
    setState({ turn, inputs: [], phase: "loading" });
    void props.read(turn)
      .then((response) => { if (current) setState({ turn, inputs: response.inputs, phase: "ready" }); })
      .catch(() => { if (current) setState({ turn, inputs: [], phase: "error" }); });
    return () => { current = false; };
  }, [open, turn, props.read, props.disabled, retry]);
  const visible = state?.turn === turn ? state : null;
  return <section className={props.className??"agent-steering-recovery"} aria-label="补充说明恢复">
    <button type="button" aria-expanded={open} onClick={() => setOpen((value) => !value)}>找回未处理的补充说明</button>
    {open && <>
      <label>已结束的轮次 <select aria-label="选择恢复轮次" value={turn ?? ""} disabled={props.disabled}
        onChange={(event) => setSelected(event.target.value)}>
        {props.turns.map((value) => <option key={value.id} value={value.id}>{value.label}</option>)}
      </select></label>
      <p>恢复会追加到当前草稿，不会自动发送。原记录保留。</p>
      {props.disabled ? <p role="status">请先结束当前运行或处理发送预览。</p>
        : !visible || visible.phase === "loading" ? <p role="status">正在读取…</p>
          : visible.phase === "error" ? <><p role="alert">读取失败，记录仍保留。请重试；持续失败时需要检查轮次状态。</p><button type="button" onClick={() => setRetry((value) => value + 1)}>重试读取</button></>
            : visible.inputs.length === 0 ? <p role="status">此轮没有未处理的补充说明。</p>
              : visible.inputs.map((input) => <div key={input.inputId}>
                <pre>{input.text}</pre>
                <button type="button" disabled={restored.has(input.inputId)} onClick={() => {
                  if (props.disabled || restoredNow.current.has(input.inputId)) return;
                  input.userInput?.length ? props.onRestore(input.text,input.userInput) : props.onRestore(input.text);
                  restoredNow.current.add(input.inputId);
                  setRestored(new Set(restoredNow.current));
                }}>{restored.has(input.inputId) ? "已恢复到草稿" : "恢复到草稿"}</button>
              </div>)}
    </>}
  </section>;
}
