import { useEffect, useState } from "react";
import { textPatchGateway, notifyTextPatchesChanged, subscribeTextPatches, type TextPatchDecision, type TextPatchReview as Review } from "./textPatchGateway";
import "./TextPatchReview.css";
import { getAppStorage } from "../../runtime/appStorage";
import { WritingTextDiff } from "./presentation/WritingTextDiff";
import { AgentSourceLink } from "./AgentSourceLink";
import { openAgentConversation } from "./openAgentConversation";
import { AgentChangeContribution } from "./AgentTurnChangeSummary";
import { openSourceLink } from "./sourceNavigation";
import { countWritingChangedLines } from "./presentation/writingChangeStats";
import { notify } from "../../services/notifications";

/** The same review component is used by chat and the note's original location. */
export function TextPatchReview({ conversationId, changeId, proposalId: legacyId, pageId, generation, presentation = "inline" }: { readonly conversationId: string; readonly changeId?: string; readonly proposalId?: string; readonly pageId?: string; readonly generation?: number; readonly presentation?: "inline" | "chat" }): React.JSX.Element | null {
  const proposalId = changeId ?? legacyId;
  const [review, setReview] = useState<Review | null>(null), [error, setError] = useState<string>(), [busy, setBusy] = useState(false);
  const [request, setRequest] = useState<TextPatchDecision>();
  const [refresh, setRefresh] = useState(0);
  useEffect(() => subscribeTextPatches(() => setRefresh(value => value + 1)), []);
  useEffect(() => {
    let active = true;
    void textPatchGateway.review(conversationId, proposalId, generation).then(result => { if (active) setReview(result); })
      .catch(reason => { if (active) setError(String(reason)); });
    return () => { active = false; };
  }, [conversationId, proposalId, generation, refresh]);
  const storageKey = review ? `agent-text-patch-decision:${conversationId}/${review.proposalId}` : undefined;
  useEffect(() => {
    if (!storageKey || !review) return;
    try {
      const raw = getAppStorage().getItem(storageKey);
      if (!raw) { setRequest(undefined); return; }
      const saved = JSON.parse(raw) as Partial<TextPatchDecision>;
      if (saved.conversationId !== conversationId || saved.proposalId !== review.proposalId || typeof saved.requestId !== "string"
        || typeof saved.generation !== "number" || !["accept", "reject", "undo"].includes(saved.action ?? "")) throw new Error("原操作恢复记录无效");
      setRequest(saved as TextPatchDecision);
    } catch (reason) { setError(String(reason)); }
  }, [conversationId, storageKey, review]);
  const decide = async (action: TextPatchDecision["action"], original?: TextPatchDecision): Promise<boolean> => {
    if (!review || busy) return false;
    const next = original ?? { conversationId, proposalId: review.proposalId, generation: review.operationGeneration ?? review.generation, requestId: crypto.randomUUID(), action };
    setRequest(next); setBusy(true); setError(undefined);
    try {
      if (!storageKey) throw new Error("审阅身份尚未就绪");
      getAppStorage().setItem(storageKey, JSON.stringify(next));
      const receipt = await textPatchGateway.decide(next);
      if (receipt.requestId !== next.requestId || receipt.proposalId !== next.proposalId || receipt.action !== next.action) throw new Error("修改结果身份不匹配；请恢复原操作。");
      getAppStorage().removeItem(storageKey); setRequest(undefined); notifyTextPatchesChanged();return true;
    } catch (reason) {
      const failure = reason && typeof reason === "object" && "status" in reason && "message" in reason ? reason : undefined;
      const knownConflict = failure?.status === "failed";
      const cause = failure && typeof failure.message === "string" ? failure.message : reason instanceof Error ? reason.message : String(reason);
      if (knownConflict) {
        if (storageKey) getAppStorage().removeItem(storageKey);
        setRequest(undefined); notifyTextPatchesChanged();
      }
      if (presentation === "chat") {
        const code = reason && typeof reason === "object" && "code" in reason && typeof reason.code === "string" ? reason.code : undefined;
        notify.error(knownConflict ? `撤销未应用：${cause}` : `无法确认撤销结果：${cause}。恢复时使用原请求，不会重复提交。`, {
          description: [`会话：${conversationId}`, `修改：${next.proposalId}`, `请求：${next.requestId}`, ...(code ? [`错误码：${code}`] : [])].join("\n"),
        });
      } else setError(knownConflict ? "审阅未应用，版本或授权已变化。请保留当前输入并核对这组修改。"
        : `无法确认操作结果：${cause}。恢复时使用原请求，不会重复提交。`);
      return false;
    }
    finally { setBusy(false); }
  };
  if (!review || (pageId && !review.pages.some(page => page.pageId === pageId))) return error ? <p role="alert">{error}</p> : null;
  const shown = pageId ? review.pages.filter(page => page.pageId === pageId) : review.pages;
  const status = { pending: review.persistenceVersion === 2 ? "已保存 · 待审阅" : "待接受",
    accept: review.persistenceVersion === 2 ? "已确认" : "已保存", undo: "已撤销", reject: "已拒绝" }[review.state];
  const warning = !review.baselineMatches && review.state !== "reject" && <p role="status">正式版本已变化，请保留手动编辑并重新读取后调整提案。</p>;
  const buttonClass = presentation === "chat" ? "writing-agent-inline-diff-undo" : undefined;
  const controls = <>{review.state === "pending" && <><button className={buttonClass} disabled={busy || !!request || !review.baselineMatches} onClick={() => void decide("accept")} type="button">接受整组</button>
      <button className={buttonClass} disabled={busy || !!request} onClick={() => void decide("reject")} type="button">拒绝整组</button>
      <button className={buttonClass} disabled={busy || !!request} onClick={() => void openAgentConversation(conversationId).catch(reason => setError(String(reason)))} type="button">继续调整</button></>}
      {review.state === "accept" && <button className={buttonClass} disabled={busy || !!request || !review.baselineMatches} onClick={() => void decide("undo")} type="button">撤销整组</button>}
      {request && !busy && <button className={buttonClass} onClick={() => void decide(request.action, request)} type="button">恢复原操作</button>}
  </>;
  if (presentation === "chat") {
    const items = shown.map(page => ({ id: page.pageId, identity:`source://notes/${page.pageId}`, label: `${page.title}${page.kind === "update" ? "" : ` · ${{ create: "新建", trash: "移入回收站", move: "移动" }[page.kind]}`}`,
      stats: countWritingChangedLines(page.before ?? "", page.after ?? ""), onOpen: () => openSourceLink(`source://notes/${page.pageId}`) }));
    const totals = items.reduce((total, item) => ({ additions: total.additions + item.stats.additions, deletions: total.deletions + item.stats.deletions }), { additions: 0, deletions: 0 });
    const canUndo = review.state === "pending" || review.state === "accept";
    const undo = canUndo?{busy,disabled:busy || !!request || review.wholeUndoAvailable === false || ((review.state === "accept" || review.persistenceVersion === 2) && !review.baselineMatches),
      run:()=>decide(review.state === "pending" ? "reject" : "undo")}:undefined;
    return <><AgentChangeContribution ariaLabel="笔记变更审阅" stats={totals} items={items} undo={undo}>
      {request && !busy && <button className="writing-agent-inline-diff-undo" onClick={() => void decide(request.action, request)} type="button">恢复原操作</button>}
    </AgentChangeContribution>{error && <p role="alert">{error}</p>}</>;
  }
  return <section className="agent-text-patch-review" aria-label="笔记变更审阅">
    <header><strong>笔记修改 · {status}</strong><span>{review.pages.length} 个关联页面</span></header>
    {warning}{shown.map(page => <PageDiff key={page.pageId} page={page} expanded={!!pageId} />)}
    <footer>{controls}</footer>{error && <p role="alert">{error}</p>}
  </section>;
}
function PageDiff({ page, expanded }: { readonly page: Review["pages"][number]; readonly expanded: boolean }): React.JSX.Element {
  const [open, setOpen] = useState(expanded);
  return <details open={open} onToggle={event => setOpen(event.currentTarget.open)}><summary>{page.title} · {{ create: "新建", trash: "移入回收站", move: "移动", update: "正文及关联变化" }[page.kind]}</summary>
    {page.kind !== "create" && <AgentSourceLink path={`source://notes/${page.pageId}`}>打开笔记</AgentSourceLink>}
    {open && <WritingTextDiff before={page.before ?? ""} after={page.after ?? ""} />}
  </details>;
}
