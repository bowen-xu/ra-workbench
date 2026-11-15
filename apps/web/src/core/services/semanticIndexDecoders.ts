import type {
  EmbeddingJobState,
  EmbeddingJobSummary,
  SemanticIndexGenerationSummary,
  SemanticIndexReconciliationSummary,
  SemanticIndexSpaceStatus,
  SemanticIndexStatus,
} from "@research-assistant/data";

type JsonObject = Readonly<Record<string, unknown>>;

const JOB_STATES: readonly EmbeddingJobState[] = [
  "queued", "running", "succeeded", "failed", "canceled", "interrupted",
];
const GENERATION_STATES = ["building", "ready", "active"] as const;
const NAMESPACES = [
  "accepted_concepts", "candidate_staging", "definition_claims", "source_passages",
] as const;
const RECONCILIATION_STATES = [
  "running", "succeeded", "failed", "canceled", "interrupted",
] as const;
const ALL_GENERATION_STATES = [
  "building", "ready", "active", "superseded", "failed",
] as const;

function objectValue(value: unknown): JsonObject | undefined {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as JsonObject : undefined;
}

function stringValue(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function optionalString(value: unknown): string | undefined | null {
  return value === undefined ? undefined : stringValue(value) ?? null;
}

function integer(value: unknown, minimum: number): number | undefined {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= minimum
    ? value : undefined;
}

function oneOf<const T extends readonly string[]>(
  value: unknown,
  values: T,
): T[number] | undefined {
  return typeof value === "string" && values.includes(value) ? value as T[number] : undefined;
}

export function decodeEmbeddingJobSummary(value: unknown): EmbeddingJobSummary | undefined {
  const item = objectValue(value);
  if (item === undefined) return undefined;
  const jobId = stringValue(item.jobId);
  const representationId = stringValue(item.representationId);
  const subjectKind = stringValue(item.subjectKind);
  const subjectId = stringValue(item.subjectId);
  const status = oneOf(item.status, JOB_STATES);
  const attempt = integer(item.attempt, 1);
  const errorCode = optionalString(item.errorCode);
  const createdAt = stringValue(item.createdAt);
  const finishedAt = optionalString(item.finishedAt);
  if (jobId === undefined || representationId === undefined || subjectKind === undefined
    || subjectId === undefined || status === undefined || attempt === undefined
    || errorCode === null || createdAt === undefined || finishedAt === null) return undefined;
  return {
    jobId, representationId, subjectKind, subjectId, status, attempt, createdAt,
    ...(errorCode === undefined ? {} : { errorCode }),
    ...(finishedAt === undefined ? {} : { finishedAt }),
  };
}

export function decodeSemanticIndexSpaceStatus(
  value: unknown,
): SemanticIndexSpaceStatus | undefined {
  const item = objectValue(value);
  if (item === undefined) return undefined;
  const embeddingSpaceId = stringValue(item.embeddingSpaceId);
  const providerId = stringValue(item.providerId);
  const modelId = stringValue(item.modelId);
  const modelRevision = stringValue(item.modelRevision);
  const dimension = integer(item.dimension, 1);
  const generation = integer(item.generation, 1);
  const generationStatus = oneOf(item.generationStatus, GENERATION_STATES);
  const namespace = oneOf(item.namespace, NAMESPACES);
  const readyCount = integer(item.readyCount, 0);
  const pendingCount = integer(item.pendingCount, 0);
  const failedCount = integer(item.failedCount, 0);
  const staleCount = integer(item.staleCount, 0);
  if (embeddingSpaceId === undefined || providerId === undefined || modelId === undefined
    || modelRevision === undefined || dimension === undefined || generation === undefined
    || generationStatus === undefined || namespace === undefined || readyCount === undefined
    || pendingCount === undefined || failedCount === undefined || staleCount === undefined) {
    return undefined;
  }
  return {
    embeddingSpaceId, providerId, modelId, modelRevision, dimension, generation,
    generationStatus, namespace, readyCount, pendingCount, failedCount, staleCount,
  };
}

export function decodeSemanticIndexStatus(value: unknown): SemanticIndexStatus | undefined {
  const item = objectValue(value);
  if (item === undefined || !Array.isArray(item.spaces) || !Array.isArray(item.runningJobIds)
    || typeof item.reconciliationRequired !== "boolean") return undefined;
  const spaces = item.spaces
    .map(decodeSemanticIndexSpaceStatus)
    .filter((space): space is SemanticIndexSpaceStatus => space !== undefined);
  const runningJobIds = item.runningJobIds
    .map(stringValue)
    .filter((id): id is string => id !== undefined);
  return { spaces, runningJobIds, reconciliationRequired: item.reconciliationRequired };
}

export function decodeSemanticIndexReconciliationSummary(
  value: unknown,
): SemanticIndexReconciliationSummary | undefined {
  const item = objectValue(value);
  if (item === undefined) return undefined;
  const runId = stringValue(item.runId);
  const status = oneOf(item.status, RECONCILIATION_STATES);
  const checkedCount = integer(item.checkedCount, 0);
  const missingCount = integer(item.missingCount, 0);
  const orphanCount = integer(item.orphanCount, 0);
  const repairedCount = integer(item.repairedCount, 0);
  const errorCode = optionalString(item.errorCode);
  const startedAt = stringValue(item.startedAt);
  const finishedAt = optionalString(item.finishedAt);
  if (runId === undefined || status === undefined || checkedCount === undefined
    || missingCount === undefined || orphanCount === undefined || repairedCount === undefined
    || errorCode === null || startedAt === undefined || finishedAt === null) return undefined;
  return {
    runId, status, checkedCount, missingCount, orphanCount, repairedCount, startedAt,
    ...(errorCode === undefined ? {} : { errorCode }),
    ...(finishedAt === undefined ? {} : { finishedAt }),
  };
}

export function decodeSemanticIndexGenerationSummary(
  value: unknown,
): SemanticIndexGenerationSummary | undefined {
  const item = objectValue(value);
  if (item === undefined) return undefined;
  const generationId = stringValue(item.generationId);
  const embeddingSpaceId = stringValue(item.embeddingSpaceId);
  const namespace = oneOf(item.namespace, NAMESPACES);
  const generation = integer(item.generation, 1);
  const status = oneOf(item.status, ALL_GENERATION_STATES);
  const recordCount = integer(item.recordCount, 0);
  const readyCount = integer(item.readyCount, 0);
  const createdAt = stringValue(item.createdAt);
  const activatedAt = optionalString(item.activatedAt);
  if (generationId === undefined || embeddingSpaceId === undefined || namespace === undefined
    || generation === undefined || status === undefined || recordCount === undefined
    || readyCount === undefined || readyCount > recordCount || createdAt === undefined
    || activatedAt === null) return undefined;
  return {
    generationId, embeddingSpaceId, namespace, generation, status, recordCount, readyCount,
    createdAt, ...(activatedAt === undefined ? {} : { activatedAt }),
  };
}
