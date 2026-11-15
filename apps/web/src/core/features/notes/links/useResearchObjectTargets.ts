import { useEffect, useMemo, useRef, useState } from "react";

import { getLiteratureGateway } from "../../../services/sourceDomainGateways";

import type {ResearchObjectTarget} from "@research-assistant/workbench-sdk/note-metadata";
export type {ResearchObjectTarget, ResearchObjectTargetKind} from "@research-assistant/workbench-sdk/note-metadata";

export function useResearchObjectTargets(onError: (reason: unknown) => void,enabled=true): readonly ResearchObjectTarget[] {
  const [targets, setTargets] = useState<readonly ResearchObjectTarget[]>([]);
  const loaded=useRef<readonly ResearchObjectTarget[]|undefined>(undefined),pending=useRef<Promise<readonly ResearchObjectTarget[]>|undefined>(undefined);
  const gateways = useMemo(() => ({
    literature: getLiteratureGateway(),
  }), []);

  useEffect(() => {
    if(!enabled)return;
    let active = true;
    if(loaded.current){setTargets(loaded.current);return;}
    const query=pending.current??=gateways.literature.list().then((papers) => {
      const result=[
        ...papers.map((paper) => ({ id: paper.id, kind: "paper" as const, label: paper.title })),
      ].sort((left, right) => left.kind.localeCompare(right.kind) || left.label.localeCompare(right.label, "zh-CN"));
      loaded.current=result;return result;
    });
    void query.then(value=>{if(active)setTargets(value);}).catch(reason=>{if(pending.current===query)pending.current=undefined;if(active)onError(reason);});
    return () => { active = false; };
  }, [gateways, onError,enabled]);

  return targets;
}
