import { AlertCircle, Check, ChevronRight, LoaderCircle, Terminal } from "lucide-react";
import { Fragment, memo, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode, type RefObject } from "react";
import { agentFailureHint } from "../../../public/agentPresentation/agentFailureHint";
import {useNestedScrollRegion} from "../../../public/layout";
import { Shimmer } from "../../../public/ui";
import { useVisibleLayoutMeasurement, visibleLayoutRect } from "../../../public/layout";

import {mergeToolReadRows} from "./toolReadActivity";
import { ToolActivityHistoryRow } from "./ToolActivityHistoryRow";
import { ToolActivityGroupIcon } from "./ToolActivityIcon";
import { toolActivityRows, toolActivitySummary } from "./toolActivityModel";
import {toolDiscoveryText} from "./toolDiscoveryActivity";
import {ToolDiscoveryTitle} from "./ToolDiscoveryTitle";
import {useTranscriptDisclosure} from "./TranscriptDisclosureState";
import {useTranscriptWindow} from "./useTranscriptWindow";

export interface WritingAgentActivityProps {
  readonly callId?: string;
  readonly turnFinished?: boolean;
  readonly interrupted?: boolean;
  readonly content: string;
  readonly error?: boolean;
  readonly name?: string;
  readonly streaming?: boolean;
  readonly type: "tool-call" | "tool-result";
  /** A later timeline group owns the actual receipt. */
  readonly laterResult?: "success" | "error" | "interrupted";
  readonly showErrorHint?: boolean;
}

export interface WritingAgentActivityGroupProps {
  readonly disclosureId?: string | undefined;
  readonly items: readonly WritingAgentActivityProps[];
  readonly headerActions?: ReactNode;
}

function displayName(name: string | undefined): string {
  if (!name) return "工具";
  const labels: Record<string, string> = {
    apply_patch: "应用修改",
    research_apply_patch: "应用修改",
    research_deliver_files: "保存文件修改",
    compile_latex: "编译 LaTeX",
    research_compile_workspace: "编译工作区",
    list_files: "查看文件",
    research_list_workspace: "查看文件",
    read_source: "读取稿件",
    search_sources: "搜索资料",
    research_review_workspace_changes: "查看文件变更",
    research_read_workspace_change: "读取文件变更",
    shell: "运行命令",
    research_exec_command: "运行命令",
    research_load_skill: "加载技能",
    research_read_skill_resource: "读取技能资源",
  };
  return labels[name] ?? name;
}

function formatActivityContent(content: string): string {
  try {
    return JSON.stringify(JSON.parse(content), null, 2);
  } catch {
    return content;
  }
}

function changeSummary(content: string): string | undefined {
  try {
    const value = JSON.parse(content) as { changes?: unknown };
    if (!Array.isArray(value.changes) || value.changes.length === 0) return undefined;
    const paths = value.changes.flatMap((change) => typeof change === "object" && change !== null && "path" in change && typeof change.path === "string" ? [change.path] : []);
    if (paths.length === 0) return `${value.changes.length} 个文件`;
    return `${paths.length} 个文件 · ${paths.join("、")}`;
  } catch {
    return undefined;
  }
}

function resultSummary(item: WritingAgentActivityProps): string | undefined {
  if (item.type !== "tool-result" || item.error) return undefined;
  if (item.name === "research_load_skill") {
    try {
      const value = JSON.parse(item.content) as { skill?: { name?: unknown } };
      return typeof value.skill?.name === "string" ? value.skill.name : undefined;
    } catch { return undefined; }
  }
  return changeSummary(item.content);
}

export function WritingAgentActivity({ content, error = false, name, streaming = false, type, showErrorHint = true }: WritingAgentActivityProps): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const label = displayName(name);
  const active = streaming && !error;
  const title = type === "tool-call"
    ? active ? `工作中 · ${label}` : `已完成 · ${label}`
    : error ? `工具失败 · ${label}` : `工具结果 · ${label}`;
  const StatusIcon = error ? AlertCircle : active ? LoaderCircle : type === "tool-result" ? Check : Terminal;
  const changed = resultSummary({ content, error, type, ...(name ? { name } : {}) });

  return <details
    className={`writing-agent-activity${error ? " error" : ""}${active ? " active" : ""}`}
    data-agent-activity={type}
    open={open}
    onToggle={(event) => setOpen(event.currentTarget.open)}
  >
    <summary aria-label={`${open ? "收起" : "展开"}${title}`}>
      <StatusIcon aria-hidden="true" className={`writing-agent-activity-status${active ? " spinning" : ""}`} size={14} />
      <span className="writing-agent-activity-title">{title}{changed ? ` · ${changed}` : ""}</span>
      <ChevronRight aria-hidden="true" className="writing-agent-activity-chevron" size={14} />
    </summary>
    {showErrorHint && error && agentFailureHint(content) && <p className="writing-agent-activity-error-hint">{agentFailureHint(content)}</p>}
    {content.trim() && <pre>{formatActivityContent(content)}</pre>}
  </details>;
}

/** One row per call, with completed history separate from the live headline. */
export function WritingAgentActivityGroup(props: WritingAgentActivityGroupProps): React.JSX.Element {
  const body = useRef<HTMLDivElement>(null);
  const [layout, setLayout] = useState("");
  useVisibleLayoutMeasurement(() => body.current?.closest<HTMLElement>(".writing-agent-messages") ?? null,
    (_element, bounds) => setLayout(`${bounds.width}:${bounds.height}`));
  return <MemoActivityGroup {...props} bodyRef={body} layout={layout}/>;
}
const MemoActivityGroup = memo(function WritingAgentActivityGroup({ items, headerActions, disclosureId, bodyRef }: WritingAgentActivityGroupProps & {
  readonly bodyRef: RefObject<HTMLDivElement | null>; readonly layout: string;
}): React.JSX.Element {
  const [open,setOpen]=useTranscriptDisclosure(disclosureId ? `group:${disclosureId}` : undefined);
  const rows=mergeToolReadRows(toolActivityRows(items)), running=rows.filter(row=>row.status==="running");
  const history=rows.filter(row=>row.status!=="running");
  const historyWindow=useTranscriptWindow(history.map(row=>({id:row.key,messages:[],row})), undefined,
    {selector:".writing-agent-activity-group-body",estimate:24,fromEnd:false,enabled:rows.length>1});
  const active=running.length>0, error=rows.some(row=>row.status==="error");
  const current=running.at(-1);
  const parallel=running.length>1 ? ` · 另有 ${running.length-1} 项运行中` : "";
  const discovery=current&&toolDiscoveryText(current);
  const title=current ? (discovery??[current.label,current.detail].filter(Boolean).join(" · "))+parallel : toolActivitySummary(rows);
  const visibleTitle=discovery&&current?<><ToolDiscoveryTitle row={current}/>{parallel}</>:title;
  const body=bodyRef, [fade,setFade]=useState(false), [fadeTop,setFadeTop]=useState(false);
  const [historyHeight,setHistoryHeight]=useState(280);
  useNestedScrollRegion(body,rows.length>1);
  function measure(){const node=body.current;
    const viewport=node?.closest<HTMLElement>(".writing-agent-messages");
    if(!visibleLayoutRect(viewport??null)||!visibleLayoutRect(node))return;
    if(viewport)setHistoryHeight(Math.max(48,Math.min(280,Math.floor(viewport.clientHeight*.6))));
    setFadeTop(!!node && node.scrollTop>1);
    setFade(!!node && node.scrollHeight-node.clientHeight-node.scrollTop>1);}
  useVisibleLayoutMeasurement(()=>body.current?.closest<HTMLElement>(".writing-agent-messages")??null,measure);
  useLayoutEffect(()=>{
    measure();const node=body.current;
    const observer=typeof ResizeObserver==="undefined"?null:new ResizeObserver(measure);
    if(node){observer?.observe(node);for(const child of node.children)observer?.observe(child);
      const viewport=node.closest(".writing-agent-messages");if(viewport)observer?.observe(viewport);}
    return()=>observer?.disconnect();
  },[open,items]);
  if(rows.length===1)return <div className="writing-agent-activity-single" data-agent-activity-single><ToolActivityHistoryRow row={rows[0]!} disclosureId={disclosureId}/></div>;
  return <div className={`writing-agent-activity-group-shell${headerActions ? " has-header-actions" : ""}`}><details
    className={`writing-agent-activity-group${error ? " error" : ""}${active ? " active" : ""}`} data-agent-activity-group open={open}
    onToggle={event=>setOpen(event.currentTarget.open)}>
    <summary aria-label={`${open?"收起":"展开"}${title}`}>
      <ToolActivityGroupIcon rows={rows}/>
      <Shimmer active={active} className="writing-agent-activity-title">{visibleTitle}</Shimmer>
      <ChevronRight aria-hidden="true" className="writing-agent-activity-chevron" size={14}/>
    </summary>
    <div className="writing-agent-activity-history" data-overflow-bottom={fade} data-overflow-top={fadeTop}>
      <div className="writing-agent-activity-group-body" ref={body} onScroll={measure} style={{"--tool-history-height":`${historyHeight}px`} as CSSProperties}>
        <div ref={historyWindow.body} className="agent-transcript-window">
          {historyWindow.rows.map(({item,gap})=><Fragment key={item.id}>
            {gap>0&&<div aria-hidden="true" style={{height:gap}}/>}
            <div data-transcript-row={item.id}><ToolActivityHistoryRow row={item.row} disclosureId={disclosureId ? `${disclosureId}/${item.id}` : undefined}/></div>
          </Fragment>)}
          {historyWindow.bottom>0&&<div aria-hidden="true" style={{height:historyWindow.bottom}}/>}
        </div>
      </div>
    </div>
  </details>{headerActions && <div className="writing-agent-activity-header-actions">{headerActions}</div>}</div>;
}, (before, after) => before.layout === after.layout && before.disclosureId === after.disclosureId && before.headerActions === after.headerActions
  && before.items.length === after.items.length && before.items.every((item, index) => {
    const next = after.items[index]!;
    return item.callId === next.callId && item.content === next.content && item.name === next.name && item.type === next.type
      && item.error === next.error && item.streaming === next.streaming && item.turnFinished === next.turnFinished
      && item.interrupted === next.interrupted && item.laterResult === next.laterResult && item.showErrorHint === next.showErrorHint;
  }));
