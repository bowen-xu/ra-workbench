import {systemDocumentGateway} from "../../services/systemDocumentGateway";
import { getStructuredResourceGateway } from "../../services/structuredResourceGateway";
import { useEffect, useState } from "react";
import { notePageId } from "@research-assistant/core/notes";
import { parseSourceLink, resolveSourcePath } from "@research-assistant/core/source";
import { getLiteratureGateway } from "../../services/sourceDomainGateways";
import { getNotesGateway } from "../notes/gateways";
import { getWritingGateway } from "../../services/sourceDomainGateways";
import { useDeveloperMode } from "../settings/DeveloperMode";
import { sourceDirectory, type SourceObject, type SourceDirectoryGateways } from "./sourceDirectory";
import { useSourceHintGateways } from "./SourceHintGateways";
import { sourceLinkHint } from "./sourceLinkHint";
import "./SourceHintContent.css";

export interface SourceLinkInfo {readonly object?: SourceObject; readonly lines: readonly string[];}
async function resolveSourceLinkInfo(path: string, supplied?: SourceDirectoryGateways): Promise<SourceLinkInfo> {
  const link = parseSourceLink(path);
  const gateways = supplied ?? {literature: getLiteratureGateway(), notes: getNotesGateway(), writing: getWritingGateway(), documents:systemDocumentGateway, structured:getStructuredResourceGateway()};
  const notes = gateways.notes;
  const node = await resolveSourcePath(sourceDirectory(gateways), link);
  const title = node.value.kind === "highlight" && link.locator?.kind === "block" && node.value.note.pageId
    ? (await notes.loadDocument(notePageId(node.value.note.pageId))).page.title : undefined;
  return {object: node.value, lines: sourceLinkHint(node.value, link.locator, title)};
}

/** Resolve a list together so its text-file numbering uses actual source types. */
export function useSourceLinkInfos(paths: readonly string[]): ReadonlyMap<string, SourceLinkInfo> {
  const gateways = useSourceHintGateways();
  const key = JSON.stringify([...new Set(paths)]);
  const [resolved, setResolved] = useState<{key: string; values: ReadonlyMap<string, SourceLinkInfo>}>();
  useEffect(() => {
    let active = true;
    const requested = JSON.parse(key) as string[];
    void Promise.all(requested.map(async path => {
      try {return [path, await resolveSourceLinkInfo(path, gateways)] as const;}
      catch {return [path, {lines: ["来源不可用"]}] as const;}
    })).then(values => {if (active) setResolved({key, values: new Map(values)});});
    return () => {active = false;};
  }, [key, gateways]);
  return resolved?.key === key ? resolved.values : new Map();
}

/** Resolves display metadata without granting access or changing navigation. */
export function useSourceLinkInfo(path: string | undefined, fallbackTitle?: string): SourceLinkInfo {
  const gateways = useSourceHintGateways();
  const [resolved, setResolved] = useState<{path: string; object: SourceObject; lines: readonly string[]}>();
  const [unavailable, setUnavailable] = useState<string>();
  useEffect(() => {
    if (!path) return;
    let active = true;
    void Promise.resolve().then(async () => {
      const info = await resolveSourceLinkInfo(path, gateways);
      if (active && info.object) setResolved({path, object: info.object, lines: info.lines});
    }).catch(() => {if (active) setUnavailable(path);});
    return () => {active = false;};
  }, [path, gateways]);
  return resolved && resolved.path === path ? {object: resolved.object, lines: resolved.lines}
    : {lines: [...(fallbackTitle ? [fallbackTitle] : []), unavailable === path ? "来源不可用" : "正在读取来源…"]};
}

export function SourceHintContent({path, lines, includeDeveloperPath = true}: {
  readonly path: string; readonly lines: readonly string[]; readonly includeDeveloperPath?: boolean;
}) {
  const {enabled: developerMode} = useDeveloperMode();
  return <span className="control-hint-lines agent-source-hint">{lines.map((line, index) =>
    <span className={index === 1 ? "agent-source-hint-location" : undefined} key={index}>{line}</span>)}
    {developerMode && includeDeveloperPath && <span className="agent-source-hint-path">{path}</span>}
  </span>;
}
