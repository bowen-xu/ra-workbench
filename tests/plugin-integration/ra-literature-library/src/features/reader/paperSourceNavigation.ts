import type {MutableRefObject} from "react";
import type {PaperAiReadingRegion} from "@research-assistant/core/ai";
import type {ReaderCommand} from "@research-assistant/core/reader";
import type {ReaderCommandPayload} from "../../../../../../../plugins/ra-literature-library/src/features/reader/commands/ReaderCommandDispatcher";
import {agentPaperContextGateway, agentPaperSubmissionGateway} from "../../../../../../apps/web/src/core/public/agent";
import {aiAssistanceErrorMessage} from "@research-assistant/workbench-sdk/ai-assistance";
import {notify} from "@research-assistant/workbench-sdk/feedback";

export async function jumpPaperSource(options: {paperId: string; conversationId: string | undefined; turnId: string; sourceId: string;
  context: Awaited<ReturnType<typeof agentPaperContextGateway.preview>> | undefined;
  cache: Map<string, Awaited<ReturnType<typeof agentPaperSubmissionGateway.sources>>>;
  identity: MutableRefObject<{paperId: string; conversationId: string | undefined}>; selection: MutableRefObject<number>;
  execute: (command: ReaderCommand, payload?: ReaderCommandPayload) => boolean;
  highlight: ((id: string, regions: readonly PaperAiReadingRegion[]) => void) | undefined}): Promise<void> {
  const {paperId, conversationId, turnId, sourceId, context, cache, identity, selection, execute, highlight} = options;
  const revision = selection.current;
  const current = context?.items.find(candidate => candidate.sourceId === sourceId || candidate.id === sourceId);
  let location: {readonly id: string; readonly regions: readonly PaperAiReadingRegion[]} | undefined = current;
  if (!current && conversationId !== undefined) {
    try {
      const key = `${conversationId}:${turnId}`;
      const result = cache.get(key) ?? await agentPaperSubmissionGateway.sources({conversationId, paperId, turnId});
      if (identity.current.paperId !== paperId || identity.current.conversationId !== conversationId) return;
      cache.set(key, result);
      const source = result.sources.find(candidate => candidate.sourceId === sourceId || candidate.id === sourceId);
      if (source) location = {id: source.id, regions: source.regions};
    } catch (reason: unknown) {
      if (selection.current === revision) notify.warning(aiAssistanceErrorMessage(reason));
      return;
    }
  }
  const region = location?.regions[0];
  if (!location || !region) {notify.warning("无法定位该论文来源"); return;}
  execute("reader.navigate", {pageIndex: region.pageIndex}); highlight?.(location.id, location.regions);
}
