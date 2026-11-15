import { useState } from "react";
import { NalQueryEditorContent } from "../../../../../../plugins/literature_library/src/features/nal/NalQueryEditorContent";
import { validQueryConditions } from "../../../../../../plugins/literature_library/src/features/nal/nalQueryBuilderModel";
import { useNalQueryDraft } from "../../../../../../plugins/literature_library/src/features/nal/useNalQueryDraft";
import { nalEditorScenarios, nalEditorState, nalEditorVocabulary, type NalEditorScenario } from "./nalEditorScenarios";
import "../../../../../../plugins/literature_library/src/features/nal/Nal.css";
import "./NalEditorPreview.css";
import { ApplicationKeyboardPolicy } from "../../../core/runtime/ApplicationKeyboardPolicy";

function EditorExample({ scenario }: { readonly scenario: NalEditorScenario }): React.JSX.Element {
  const draft = useNalQueryDraft({ conditions: scenario.create(), outputs: undefined });
  const { conditions, setConditions, outputs, setOutputs } = draft;
  return <>
    <section className="nal-preview-stage" aria-label="嵌入式语句编辑示例">
      <div className="nal-query-editor">
        <NalQueryEditorContent conditions={conditions} onChange={setConditions} state={nalEditorState}
          vocabulary={nalEditorVocabulary} outputVariables={outputs} onOutputVariablesChange={setOutputs} history={draft} />
      </div>
    </section>
    <p className="nal-preview-status" role="status">{validQueryConditions(conditions) ? "语句结构已完整" : "还有待选择的连接符或词项"}</p>
  </>;
}

export function NalEditorPreview(): React.JSX.Element {
  const [scenario, setScenario] = useState(nalEditorScenarios[0]!);
  const [revision, setRevision] = useState(0);
  return <main className="nal-editor-preview"><ApplicationKeyboardPolicy />
    <header className="nal-preview-heading"><p>Narsese · 交互预览</p><h1>嵌入式语句编辑</h1>
      <p>直接在语句中编辑词项、组合与嵌套结构。</p>
    </header>
    <nav className="nal-preview-scenarios" aria-label="编辑示例">
      {nalEditorScenarios.map(example => <button type="button" key={example.id} aria-pressed={scenario.id === example.id}
        onClick={() => setScenario(example)}>{example.label}</button>)}
      <button type="button" onClick={() => setRevision(value => value + 1)}>重置当前示例</button>
    </nav>
    <p className="nal-preview-description">{scenario.description}</p>
    <EditorExample key={`${scenario.id}:${revision}`} scenario={scenario} />
    <aside className="nal-preview-guide" aria-label="操作说明">
      <p>所有选择均单击打开菜单，点选即确认；变量名按 Enter 提交，Esc 取消未提交的文字。</p>
      <p>行前 + 添加 statement 或分组，六点手柄调整 block 顺序；compound 内部的词项也可拖动。</p>
      <p>键盘排序：block 使用 Alt + ↑ / ↓，compound 内词项使用 Alt + ← / →。</p>
      <p>输出变量默认全部包含；每个标签上的 × / + 控制是否输出。</p>
      <p>撤销／重做覆盖已确认的编辑：⌘ / Ctrl + Z 撤销，⌘ / Ctrl + Shift + Z 重做；输入框内保留文本撤销。</p>
    </aside>
    <footer className="nal-preview-footer">使用正式查询编辑组件与示例词表；修改仅保留在本页，刷新后重置，不写入文献数据。</footer>
  </main>;
}
