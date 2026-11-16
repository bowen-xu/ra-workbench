import { notify } from "../../../../../../apps/web/src/core/public/feedback";
import { useEffect, useMemo, useState } from "react";
import type {
  AiCandidate,
  AiRun,
  AiRunStage,
  AiRunStatus,
  AiScopeItem,
  AiTaskKind,
} from "@research-assistant/core/ai";

import {
  aiAssistanceErrorMessage,
  getAiAssistanceGateway,
  type AiAssistanceGateway,
} from "../../../../../../apps/web/src/core/public/aiAssistance";
import "../../../../../../../plugins/ra-literature-library/src/features/ai/AiCandidateReview.css";

interface AiRunHistoryPanelProps {
  readonly gateway?: AiAssistanceGateway;
  readonly limit?: number;
  readonly projectId?: string;
}

interface RunCandidateSummary {
  readonly total: number;
  readonly pending: number;
  readonly terminal: number;
}

type RunStatusFilter = "all" | "running" | "needs_review" | "failed_or_canceled" | "terminal";

const taskLabels: Readonly<Record<AiTaskKind, string>> = {
  provider_diagnostic: "Provider 诊断",
  selection_explanation: "选区解释",
  paper_profile_classification: "生成文献画像",
  paper_overview: "论文概要",
  paper_concept_map: "论文关键概念",
  paper_suggestions: "阅读建议",
  reference_extraction: "参考文献提取",
  matrix_synthesis: "多文献综合",
};

const statusLabels: Readonly<Record<AiRunStatus, string>> = {
  draft: "草稿",
  queued: "排队中",
  resolving_scope: "解析范围",
  retrieving: "检索证据",
  extracting_evidence: "抽取证据",
  generating: "生成中",
  validating: "校验中",
  needs_review: "待审查",
  partial: "部分完成",
  failed: "失败",
  canceled: "已取消",
};

const runningStatuses = new Set<AiRunStatus>([
  "draft",
  "queued",
  "resolving_scope",
  "retrieving",
  "extracting_evidence",
  "generating",
  "validating",
]);

const terminalStatuses = new Set<AiRunStatus>(["partial", "failed", "canceled"]);

const taskFilterOptions = Object.keys(taskLabels) as readonly AiTaskKind[];

function latestStage(run: AiRun): AiRunStage | undefined {
  return [...run.stages].sort((left, right) =>
    right.createdAt.localeCompare(left.createdAt) || right.attempt - left.attempt)[0];
}

function usageText(run: AiRun): string {
  const input = run.inputTokens ?? run.stages.reduce((sum, stage) => sum + (stage.inputTokens ?? 0), 0);
  const output = run.outputTokens ?? run.stages.reduce((sum, stage) => sum + (stage.outputTokens ?? 0), 0);
  if (input === 0 && output === 0) return "无用量记录";
  return `输入 ${input} · 输出 ${output}`;
}

function stageUsageText(stage: AiRunStage): string {
  const parts = [
    stage.inputTokens === undefined ? undefined : `输入 ${stage.inputTokens}`,
    stage.outputTokens === undefined ? undefined : `输出 ${stage.outputTokens}`,
    stage.cacheHitTokens === undefined ? undefined : `cache hit ${stage.cacheHitTokens}`,
    stage.cacheMissTokens === undefined ? undefined : `cache miss ${stage.cacheMissTokens}`,
  ].filter((part): part is string => part !== undefined);
  return parts.length === 0 ? "无阶段用量" : parts.join(" · ");
}

function hashText(label: string, hash: string | undefined): string | undefined {
  return hash === undefined ? undefined : `${label} ${hash.slice(0, 12)}…`;
}

function scopeItemMetadata(item: AiScopeItem): string {
  const metadata = [
    item.parentObjectId === undefined ? undefined : `parent ${item.parentObjectId.slice(0, 12)}…`,
    item.objectRevision === undefined ? undefined : `revision ${item.objectRevision}`,
    hashText("content", item.contentSha256),
    hashText("snapshot", item.snapshotHash),
  ].filter((part): part is string => part !== undefined);
  return metadata.length === 0 ? "无版本化元数据" : metadata.join(" · ");
}

function scopeText(run: AiRun): string {
  if (run.scopeItems.length === 0) return "无范围项";
  const kinds = new Map<string, number>();
  for (const item of run.scopeItems) kinds.set(item.itemKind, (kinds.get(item.itemKind) ?? 0) + 1);
  return [...kinds.entries()].map(([kind, count]) => `${kind} ${count}`).join(" · ");
}

function cloudConsentText(run: AiRun): string {
  if (run.cloudConsentVersion !== undefined && run.cloudConsentedAt !== undefined) {
    return `已确认 ${run.cloudConsentVersion} · ${run.cloudConsentedAt}`;
  }
  if (run.providerId === "fixture-generation") return "fixture provider，无云端外发";
  return "未记录逐次云端确认";
}

function summarizeCandidates(candidates: readonly AiCandidate[]): ReadonlyMap<string, RunCandidateSummary> {
  const summaries = new Map<string, RunCandidateSummary>();
  for (const candidate of candidates) {
    const current = summaries.get(candidate.runId) ?? { total: 0, pending: 0, terminal: 0 };
    const terminal = ["accepted_unchanged", "accepted_after_edit", "rejected"].includes(candidate.reviewState);
    summaries.set(candidate.runId, {
      total: current.total + 1,
      pending: current.pending + (terminal ? 0 : 1),
      terminal: current.terminal + (terminal ? 1 : 0),
    });
  }
  return summaries;
}

function candidateSummaryText(summary: RunCandidateSummary | undefined): string {
  if (summary === undefined || summary.total === 0) return "尚无候选";
  return `${summary.total} 个候选 · ${summary.pending} 待审查 · ${summary.terminal} 已终态`;
}

function matchesStatusFilter(run: AiRun, filter: RunStatusFilter): boolean {
  if (filter === "all") return true;
  if (filter === "running") return runningStatuses.has(run.status);
  if (filter === "needs_review") return run.status === "needs_review";
  if (filter === "failed_or_canceled") return run.status === "failed" || run.status === "canceled";
  return terminalStatuses.has(run.status);
}

function orderedStages(run: AiRun): readonly AiRunStage[] {
  return [...run.stages].sort((left, right) =>
    left.createdAt.localeCompare(right.createdAt) || left.attempt - right.attempt);
}

export function AiRunHistoryPanel({
  gateway: providedGateway,
  limit = 25,
  projectId,
}: AiRunHistoryPanelProps): React.JSX.Element {
  const defaultGateway = useMemo(() => getAiAssistanceGateway(), []);
  const gateway = providedGateway ?? defaultGateway;
  const [runs, setRuns] = useState<readonly AiRun[]>([]);
  const [candidateSummaries, setCandidateSummaries] = useState<ReadonlyMap<string, RunCandidateSummary>>(new Map());
  const [loading, setLoading] = useState(true);
  const [taskFilter, setTaskFilter] = useState<AiTaskKind | "all">("all");
  const [statusFilter, setStatusFilter] = useState<RunStatusFilter>("all");
  const [busyRunId, setBusyRunId] = useState<string>();

  async function refresh(): Promise<void> {
    setLoading(true);

    try {
      const [nextRuns, nextCandidates] = await Promise.all([
        gateway.listRuns(limit),
        gateway.listCandidates(projectId),
      ]);
      setRuns(nextRuns);
      setCandidateSummaries(summarizeCandidates(nextCandidates));
    } catch (reason) {
      notify.error(aiAssistanceErrorMessage(reason));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void refresh();
  }, [gateway, limit, projectId]);

  useEffect(() => {
    let active = true;
    const interval = window.setInterval(() => {
      if (!active || runs.every((run) => !runningStatuses.has(run.status))) return;
      void refresh();
    }, 5_000);
    return () => {
      active = false;
      window.clearInterval(interval);
    };
  }, [gateway, limit, projectId, runs]);

  async function cancelRun(run: AiRun): Promise<void> {
    setBusyRunId(run.id);


    try {
      const requested = await gateway.cancelRun(run.id);
      notify.info(requested ? `已请求取消 ${run.id.slice(0, 8)}…` : `运行 ${run.id.slice(0, 8)}… 当前不可取消`);
      await refresh();
    } catch (reason) {
      notify.error(aiAssistanceErrorMessage(reason));
    } finally {
      setBusyRunId(undefined);
    }
  }

  const visibleRuns = runs.filter((run) =>
    (taskFilter === "all" || run.taskKind === taskFilter) && matchesStatusFilter(run, statusFilter));

  return (
    <section className="ai-run-history">
      <header>
        <div><span className="eyebrow">AI RUNS</span><h2>AI 运行记录</h2></div>
        <div><small>最近 {limit} 条</small><button disabled={loading} onClick={() => void refresh()} type="button">刷新</button></div>
      </header>
      <p>运行记录展示任务范围、阶段、模型、用量和错误；内容修改与审阅在所属文献或工作区完成。</p>
      <div className="ai-run-filters">
        <label>任务<select aria-label="运行任务筛选" onChange={(event) => setTaskFilter(event.target.value as AiTaskKind | "all")} value={taskFilter}>
          <option value="all">全部任务</option>
          {taskFilterOptions.map((taskKind) => <option key={taskKind} value={taskKind}>{taskLabels[taskKind]}</option>)}
        </select></label>
        <label>状态<select aria-label="运行状态筛选" onChange={(event) => setStatusFilter(event.target.value as RunStatusFilter)} value={statusFilter}>
          <option value="all">全部状态</option>
          <option value="running">运行中</option>
          <option value="needs_review">待审查</option>
          <option value="failed_or_canceled">失败或取消</option>
          <option value="terminal">已终止</option>
        </select></label>
      </div>
      {loading && <p role="status">正在读取 AI 运行记录…</p>}
      {!loading && runs.length === 0 && <div className="evidence-empty">尚无 AI 运行记录。</div>}
      {!loading && runs.length > 0 && visibleRuns.length === 0 && <div className="evidence-empty">当前筛选下没有 AI 运行记录。</div>}
      <div className="ai-run-list">
        {visibleRuns.map((run) => {
          const stage = latestStage(run);
          const canCancel = runningStatuses.has(run.status) && !run.cancelRequested;
          const candidateSummary = candidateSummaries.get(run.id);
          return (
            <article className={`ai-run-card status-${run.status}`} key={run.id}>
              <header>
                <div>
                  <strong>{taskLabels[run.taskKind]}</strong>
                  <small>{run.id.slice(0, 8)}… · {statusLabels[run.status]} · {run.createdAt}</small>
                </div>
                <div className="ai-run-actions">
                  {canCancel && <button disabled={busyRunId === run.id} onClick={() => void cancelRun(run)} type="button">{busyRunId === run.id ? "取消中…" : "取消"}</button>}
                </div>
              </header>
              <div className="ai-run-facts">
                <span>{run.providerId} / {run.modelId}</span>
                {run.sharedTurnOutcome !== undefined && <span>shared turn {run.sharedTurnOutcome}</span>}
                {run.reasoningEffort !== undefined && <span>reasoning {run.reasoningEffort}</span>}
                <span>{scopeText(run)}</span>
                <span>{candidateSummaryText(candidateSummary)}</span>
                <span>{usageText(run)}</span>
                <span>scope {run.scopeHash.slice(0, 12)}…</span>
              </div>
              {stage !== undefined && <p>当前阶段：{stage.stageKind} · {stage.status} · attempt {stage.attempt}</p>}
              <details className="ai-run-details">
                <summary>诊断详情</summary>
                <dl>
                  <div><dt>版本</dt><dd>{run.pipelineVersion} · {run.promptVersion} · schema {run.outputSchemaVersion}</dd></div>
                  <div><dt>预算</dt><dd>输入 {run.maxInputTokens} · 输出 {run.maxOutputTokens} · 超时 {run.timeoutMs} ms</dd></div>
                  <div><dt>外发确认</dt><dd>{cloudConsentText(run)}</dd></div>
                  {run.sharedConversationId !== undefined && <div><dt>Shared Agent</dt><dd>conversation {run.sharedConversationId} · request {run.sharedRequestId ?? "未知"} · turn {run.sharedTurnId ?? "未完成"} · {run.sharedTurnOutcome ?? "进行中或无终态"} · candidate {run.sharedCandidateId ?? "未生成"} · legacy {run.sharedTerminalStatus ?? "未终态"}</dd></div>}
                  <div><dt>数据保留</dt><dd>不保存 provider 原始请求/响应；仅保存规范化 trace、hash、用量和错误</dd></div>
                  <div><dt>时间</dt><dd>{run.startedAt ?? "未开始"} → {run.finishedAt ?? "未完成"} · 更新 {run.updatedAt}</dd></div>
                </dl>
                <div className="ai-run-scope-items">
                  <strong>Scope Snapshot</strong>
                  {run.scopeItems.length === 0 && <p>尚无范围项。</p>}
                  {run.scopeItems.map((item) => (
                    <section key={item.id}>
                      <header><span>{item.ordinal + 1}. {item.itemKind}</span><small>{item.objectId.slice(0, 12)}…</small></header>
                      <p>{scopeItemMetadata(item)}</p>
                    </section>
                  ))}
                </div>
                <div className="ai-run-stage-timeline">
                  <strong>阶段记录</strong>
                  {run.stages.length === 0 && <p>尚无阶段记录。</p>}
                  {orderedStages(run).map((item) => {
                    const hashes = [
                      hashText("input", item.inputHash),
                      hashText("output", item.outputHash),
                    ].filter((part): part is string => part !== undefined);
                    return (
                      <section key={item.id}>
                        <header><span>{item.stageKind} #{item.attempt}</span><small>{item.status}</small></header>
                        <p>{stageUsageText(item)}{item.durationMs === undefined ? "" : ` · ${item.durationMs} ms`}</p>
                        {hashes.length > 0 && <p>{hashes.join(" · ")}</p>}
                        {item.errorMessage !== undefined && <p className="ai-run-error">{item.errorCode ?? "stage_error"}：{item.errorMessage}</p>}
                      </section>
                    );
                  })}
                </div>
              </details>
              {run.errorMessage !== undefined && <p className="ai-run-error">{run.errorMessage}</p>}
            </article>
          );
        })}
      </div>
    </section>
  );
}
