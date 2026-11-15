import type {AgentSubmissionInput} from "@research-assistant/core/agent";
import {NoteInstructionProfilePreview} from "./NoteInstructionProfilePreview";

export function NoteFrozenSubmission({input}:{readonly input:AgentSubmissionInput}):React.JSX.Element {
  return <>
    <strong>{input.model.providerId} · {input.model.modelId}</strong>
    <p>来源：{input.context[0]?.revision} · 消息将与此冻结版本一起发送给模型。</p>
    <details><summary>查看发送内容</summary><p>{input.text}</p><pre>{input.context[0]?.text}</pre></details>
    <NoteInstructionProfilePreview input={input}/>
  </>;
}
