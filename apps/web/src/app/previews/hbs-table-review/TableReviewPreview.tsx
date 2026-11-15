import { useEffect, useState, useSyncExternalStore } from "react";
import { compareTableRows } from "@research-assistant/core/hbs";
import { HbsNoteEditor } from "../../../core/components/hbs/HbsNoteEditor";
import { createTableScenario, tableRows } from "./scenarios";
import "./preview.css";

type Scenario = Awaited<ReturnType<typeof createTableScenario>>;
export function TableReviewPreview(): React.JSX.Element {
  const [scenario, setScenario] = useState<Scenario>();
  const [error, setError] = useState("");
  const [mode, setMode] = useState<"combined" | "plain" | "duplicates">("combined");
  const [generation, setGeneration] = useState(0);
  useEffect(() => {
    let disposed = false, current: Scenario | undefined;
    void createTableScenario(mode).then(value => {
      current = value;
      if (disposed) value.session.dispose(); else { setScenario(value); setError(""); }
    }).catch(reason => { if (!disposed) setError(String(reason)); });
    return () => { disposed = true; current?.session.dispose(); };
  }, [mode, generation]);
  const reset = (next: typeof mode) => { setScenario(undefined); setMode(next); setGeneration(value => value + 1); };
  return <main className="table-review-preview">
    <header><h1>表格内部差异</h1><p>真实 BlockNote 组件 · daff 1.4.2 行对应 · 合成数据 · 内存网关，刷新重置</p></header>
    {error && <p role="alert">{error}</p>}
    {scenario ? <ScenarioView key={generation + mode} scenario={scenario} reset={reset}/> : <p>正在创建真实编辑会话…</p>}
  </main>;
}
function ScenarioView({ scenario, reset }: { scenario: Scenario; reset: (mode: "combined" | "plain" | "duplicates") => void }): React.JSX.Element {
  const state = useSyncExternalStore(scenario.session.subscribe, scenario.session.getSnapshot);
  const [error, setError] = useState("");
  const comparison = compareTableRows(tableRows(state.baselineProjection), tableRows(state.content));
  const run = (operation: () => Promise<unknown>) => { void operation().catch(reason => setError(String(reason))); };
  const inserted = comparison.pairs.filter(pair => pair.before === undefined).length;
  const removed = comparison.pairs.filter(pair => pair.after === undefined).length;
  return <>
    <nav className="table-review-preview-actions" aria-label="表格示例操作">
      <button onClick={() => reset("plain")}>重置到原表</button>
      <button onClick={() => run(scenario.editCell)}>修改 S：单元格</button>
      <button onClick={() => run(scenario.insertRow)}>插入行</button>
      <button onClick={() => run(() => scenario.insertColumn())}>插入列</button>
      <button onClick={() => run(scenario.deleteRow)}>删除有噪声行</button>
      <button onClick={() => run(() => scenario.session.save())}>保存 H</button>
      <button onClick={() => reset("combined")}>重现：先修改再插行</button>
      <button onClick={() => reset("duplicates")}>重复行新增</button>
    </nav>
    <p className="table-review-preview-status" role="status">{comparison.ambiguous ? comparison.reason : `新增 ${inserted} 行 · 删除 ${removed} 行 · 新增 ${comparison.columns?.filter(pair => pair.before === undefined).length ?? 0} 列`}
      {` · ${state.changes.length ? "有待审修改" : "无待审修改"} · ${state.conflicts.length} 处冲突`}</p>
    <p>绿色：新增行列／当前修改；红色：删除行／修改前的值。单格修改在格内审阅，行列结构变化按整表审阅；保存不等于采纳。</p>
    {error && <p role="alert">{error}</p>}
    <section className="table-review-preview-editor" aria-label="实际笔记编辑器">
      <HbsNoteEditor session={scenario.session}/>
    </section>
    <details className="table-review-preview-data"><summary>算法结果与实际操作范围</summary>
      <pre>{JSON.stringify({ correspondence: comparison, reviewRanges: state.changes.map(({ kind, path, reason }) => ({ kind, path, reason })) }, null, 2)}</pre>
    </details>
    <details className="table-review-preview-data"><summary>合成数据与消息记录</summary>
      <pre>{JSON.stringify({ before: tableRows(state.baselineProjection), current: tableRows(state.content), events: scenario.events, session: state.events }, null, 2)}</pre>
    </details>
  </>;
}
