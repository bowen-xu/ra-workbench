import { useEffect, useRef, useState } from "react";
import { createNoteDocumentAdapter, createStructuredDocumentAdapter, createTextDocumentAdapter, type HbsDocumentAdapter } from "@research-assistant/core/hbs";
import { HbsNoteEditor } from "../../../core/components/hbs/HbsNoteEditor";
import { HbsLeafEditor } from "../../../core/components/hbs/HbsLeafEditor";
import { HbsStructuredEditor } from "../../../core/components/hbs/HbsStructuredEditor";
import { HbsSnapshotSource } from "../../../core/components/hbs/HbsSnapshotSource";
import { HbsStorageRows, type HbsStoredTable } from "../../../core/components/hbs/HbsStorageRows";
import { HbsAdapterWorkspace } from "../../../core/components/hbs/HbsAdapterWorkspace";
import { directTextDiff } from "../../../core/components/hbs/codeMirrorHbsPolicy";
import { noteFixture, structuredFixture, textFixture } from "./fixtures";
import { previewCall, openExample } from "./storage";
import { changeNoteExample, noteExternalActions, type ExampleAssets } from "./noteExternalChanges";
import { resetExample } from "./resetExample";
import { HbsOperationLog } from "../../../core/components/hbs/operationLog";
import { HbsOperationLogPanel } from "../../../core/components/hbs/HbsOperationLogPanel";
const configuration = { labels: { incoming: "Remain", both: "Both" }, hints: { incoming: "remain incoming", both: "remain both" } } as const;
const adapters = { text: createTextDocumentAdapter(directTextDiff), note: createNoteDocumentAdapter(directTextDiff), structured: createStructuredDocumentAdapter(directTextDiff) };
type Kind = keyof typeof adapters;
export function HbsAdaptersPreview(): React.JSX.Element {
  const [kind, setKind] = useState<Kind>("note"), [generation, setGeneration] = useState(0), [error, setError] = useState("");
  // The discriminated render boundary retains concrete types in the reusable
  // editors; the harness stores the three kinds as a union only here.
  const [example, setExample] = useState<Awaited<ReturnType<typeof openExample<unknown>>>>();
  const [authority, setAuthority] = useState<{ version: number; saved: unknown; baseline: unknown;storage:Record<"S"|"B"|"S′",readonly HbsStoredTable[]> }>();
  const automatic=useRef(true);
  const [operations] = useState(() => new HbsOperationLog());
  const resetting = useRef(false);
  const resetBusy = useRef(false);
  const externalOperations = useRef(new Set<Promise<void>>());
  const [resetPhase, setResetPhase] = useState<"idle" | "resetting" | "failed">("idle");
  const fixture = kind === "note" ? noteFixture : kind === "text" ? textFixture : structuredFixture;
  useEffect(() => {
    let active = true; let current: typeof example;
    resetting.current = false; setResetPhase("idle");
    setExample(undefined); setAuthority(undefined); setError("");
    void openExample(adapters[kind] as HbsDocumentAdapter<unknown>, fixture).then(async value => {
      current = value; if (!active) { value.session.dispose(); value.delivery.dispose(); return; }
      value.delivery.setAutomatic(automatic.current);
      const inspected=await value.inspect();
      if(active){setExample(value);setAuthority(inspected);}
    }).catch(reason => { if (active) setError(String(reason)); });
    return () => { active = false;if(current)automatic.current=current.delivery.getSnapshot().automatic;current?.session.dispose(); current?.delivery.dispose(); };
  }, [kind, generation]);
  useEffect(() => {
    if (!example) return;
    // Observe the actual authority after real receipts, without pushing it into
    // H. The sync button controls delivery of external snapshots to the client.
    let active=true; let inspection=0;
    const refresh=()=>{const requested=++inspection;void example.inspect().then(value=>{if(active && !resetting.current && requested===inspection)setAuthority(value);}).catch(reason=>{if(active && !resetting.current && requested===inspection)setError(String(reason));});};
    let preserved=example.session.getSnapshot().preserved;
    const stopDelivery=example.delivery.subscribe(refresh);
    const stopSession=example.session.subscribe(()=>{const next=example.session.getSnapshot().preserved;if(next!==preserved){preserved=next;refresh();}});
    return()=>{active=false;stopDelivery();stopSession();};
  }, [example]);
  async function external(id: string): Promise<void> {
    if (!example) return;
    try {
      const assets = kind === "note" && (id === "image" || id === "file")
        ? await previewCall("note", "fixtureAssets", example.id) as ExampleAssets : undefined;
      const current = await example.actual.read();
      const position = id === "second";
      const saved = structuredClone(current.saved) as typeof noteFixture & typeof structuredFixture;
      let next: unknown = saved;
      if (kind === "text") next = position ? String(saved).replace(/下一步：[^\n]*/u, `下一步：增加对照实验 v${current.version + 1}。`)
        : String(saved).replace(/准确率为 [^。]*/u, `准确率为 ${80 + current.version * 5}%`);
      else if (kind === "note") next = changeNoteExample(adapters.note.decode(current.saved), id, current.version + 1, assets);
      else saved.fields[position ? 1 : 0]!.statements[0]!.text = `${position ? "比较有噪声的实验结果" : "外部更新的研究问题"} v${current.version + 1}。`;
      const result = await example.actual.commit({ id: crypto.randomUUID(), expectedVersion: current.version, saved: next, baseline: current.baseline });
      operations.record("外部修改已提交", {kind, resourceId: example.id, action: id, before: current, after: result});
      example.session.record(`外部写入 S：${noteExternalActions.find(action=>action.id===id)?.label ?? id} · v${result.version}；尚未同步到 H。`);
      setAuthority(await example.inspect()); setError("");
    } catch (reason) { operations.record("外部修改失败", {kind, resourceId: example.id, action: id, error: String(reason)}); setError(String(reason)); example.session.record(String(reason)); }
  }
  function runExternal(id: string): void {
    if (resetting.current) return;
    const operation = external(id);
    externalOperations.current.add(operation);
    void operation.finally(() => externalOperations.current.delete(operation));
  }
  async function reset(): Promise<void> {
    if (!example || resetBusy.current) return;
    resetBusy.current = true;
    resetting.current = true; setResetPhase("resetting"); setError("");
    operations.record("开始重置", {kind, resourceId: example.id, before: example.session.getSnapshot(), transport: example.delivery.getSnapshot()});
    try {
      await resetExample({ ...example, adapter: adapters[kind] as HbsDocumentAdapter<unknown>, fixture,
        external: [...externalOperations.current],
        clearPreservation: async () => { await previewCall(kind, "clearPreservation", example.id); },
        readPreservation: () => previewCall(kind, "readPreservation", example.id),
      });
      operations.clear();
      setGeneration(value => value + 1);
    } catch (reason) {
      setError(`MODEL_RESET_FAILED：${reason instanceof Error ? reason.message : String(reason)}`);
      operations.record("重置失败", {kind, resourceId: example.id, error: String(reason), transport: example.delivery.getSnapshot()});
      setResetPhase("failed");
    } finally { resetBusy.current = false; }
  }
  const controls = <label>文档类型 <select disabled={resetPhase === "resetting"} aria-label="文档类型" value={kind} onChange={event => setKind(event.target.value as Kind)}>
    <option value="note">BlockNote 笔记</option><option value="structured">结构化资源</option><option value="text">文本</option>
  </select></label>;
  if (resetPhase !== "idle") return <main className="hbs-connection-error">{controls}<p role={resetPhase === "failed" ? "alert" : "status"}>{resetPhase === "failed" ? error : "正在重置当前资源：等待旧写入完成并核对初始数据…"}</p>{resetPhase === "failed" && <button onClick={() => { void reset(); }}>重试重置当前资源</button>}<HbsOperationLogPanel log={operations}/></main>;
  if (!example || !authority || example.session.adapter.kind!==kind) return <main className="hbs-connection-error">{controls}<p role={error ? "alert" : "status"}>{error || "正在打开 IndexedDB 存储…"}</p><button onClick={() => setGeneration(value => value + 1)}>重新连接</button></main>;
  return <>{error && <p role="alert">{error}</p>}<HbsAdapterWorkspace key={`${kind}:${generation}`} session={example.session} delivery={example.delivery} authoritative={authority}
    operations={operations} resourceId={example.id}
    controls={controls} externalActions={kind === "note" ? noteExternalActions : [{id:"first",label:"修改 S：第一处"},{id:"second",label:"修改 S：第二处"}]}
    onExternal={runExternal} onRestart={() => {void example.session.flushPreservation().then(()=>{if (!resetting.current) setGeneration(value=>value+1);});}} onReset={() => { void reset(); }}
    renderSnapshot={(value,label) => label==="B′" ? <HbsSnapshotSource value={{storage:"未落库，由 B 与普通输入投影推导",value}} /> : <HbsStorageRows tables={authority.storage[label as "S"|"B"|"S′"]} />}
    renderEditor={() => kind === "note" ? <HbsNoteEditor session={example.session as import("@research-assistant/core/hbs").HbsSession<typeof noteFixture>} configuration={configuration} />
      : kind === "structured" ? <HbsStructuredEditor session={example.session} configuration={configuration} />
        : <HbsLeafEditor session={example.session} path={[]} label="H.txt" configuration={configuration} />} /></>;
}
