export interface StructuredChangeTarget {
  readonly resourceId: string;
  readonly revision: number;
}
export interface TextPatchTarget {
  readonly path: string;
  readonly name?: string | null;
  readonly status: "applied" | "proposed" | "failed" | "unknown";
  readonly message: string;
  readonly changeId?: string;
  readonly generation?: number;
  readonly structuredResource?: StructuredChangeTarget;
}
function record(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : undefined;
}
/** Normalize persisted legacy receipts at the boundary, including messages
 * written before structured-resource metadata became a separate property. */
export function textPatchTargets(content: string): readonly TextPatchTarget[] {
  try {
    const targets = record(JSON.parse(content))?.targets;
    if (!Array.isArray(targets)) return [];
    return targets.flatMap((value: unknown): TextPatchTarget[] => {
      const target = record(value);
      if (!target || typeof target.path !== "string" || typeof target.message !== "string"
        || !["applied", "proposed", "failed", "unknown"].includes(String(target.status))) return [];
      let structured = record(target.structuredResource);
      if (!structured) {
        try { structured = record(JSON.parse(target.message)); } catch { /* Ordinary tool text. */ }
      }
      const receipt = record(structured?.receipt);
      const structuredResource = structured && typeof structured.resourceId === "string"
        && Number.isSafeInteger(structured.revision) && Number(structured.revision) > 0
        ? { resourceId: structured.resourceId, revision: Number(structured.revision) } : undefined;
      const identity = target.changeId ?? target.proposalId
        ?? (target.status === "applied" ? receipt?.requestId : undefined);
      return [{ path: target.path, status: target.status as TextPatchTarget["status"], message: target.message,
        ...(typeof target.name === "string" ? { name: target.name } : {}),
        ...(typeof identity === "string" && identity ? { changeId: identity } : {}),
        ...(typeof target.generation === "number" ? { generation: target.generation } : {}),
        ...(structuredResource ? { structuredResource } : {}),
      }];
    });
  } catch { return []; }
}
