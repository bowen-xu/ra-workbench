import type {
  ConceptRetrievalEdge,
  ConceptRetrievalPreview,
  ConceptRetrievalSeed,
  ConceptRetrievalSource,
} from "@research-assistant/core/ai";

type JsonRecord = Readonly<Record<string, unknown>>;

function record(value: unknown): JsonRecord | undefined {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as JsonRecord : undefined;
}

function string(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function boolean(value: unknown): boolean | undefined {
  return typeof value === "boolean" ? value : undefined;
}

function integer(value: unknown): number | undefined {
  return Number.isSafeInteger(value) ? value as number : undefined;
}

function number(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function optionalString(value: unknown): string | undefined | null {
  return value === undefined ? null : string(value);
}

function optionalNumber(value: unknown): number | undefined | null {
  return value === undefined ? null : number(value);
}

function oneOf<const Values extends readonly string[]>(
  value: unknown,
  values: Values,
): Values[number] | undefined {
  return typeof value === "string" && values.includes(value)
    ? value as Values[number] : undefined;
}

function list<T>(value: unknown, decode: (item: unknown) => T | undefined): readonly T[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const decoded = value.map(decode);
  return decoded.some((item) => item === undefined) ? undefined : decoded as readonly T[];
}

function seed(value: unknown): ConceptRetrievalSeed | undefined {
  const data = record(value);
  if (data === undefined) return undefined;
  const conceptId = string(data.conceptId); const conceptRevision = integer(data.conceptRevision);
  const label = string(data.label); const preferredTerm = string(data.preferredTerm);
  const matchedBy = list(data.matchedBy, (item) => oneOf(item, ["lexical", "vector"] as const));
  const lexicalRank = optionalNumber(data.lexicalRank); const vectorRank = optionalNumber(data.vectorRank);
  const vectorDistance = optionalNumber(data.vectorDistance); const rrfScore = number(data.rrfScore);
  if (conceptId === undefined || conceptRevision === undefined || conceptRevision < 1
    || label === undefined || preferredTerm === undefined || matchedBy === undefined
    || matchedBy.length === 0 || lexicalRank === undefined || vectorRank === undefined
    || vectorDistance === undefined || rrfScore === undefined || rrfScore <= 0) return undefined;
  return { conceptId, conceptRevision, label, preferredTerm, matchedBy, rrfScore,
    ...(lexicalRank === null ? {} : { lexicalRank }),
    ...(vectorRank === null ? {} : { vectorRank }),
    ...(vectorDistance === null ? {} : { vectorDistance }) };
}

function edge(value: unknown): ConceptRetrievalEdge | undefined {
  const data = record(value);
  if (data === undefined) return undefined;
  const relationAssertionId = string(data.relationAssertionId);
  const relationRevision = integer(data.relationRevision);
  const subjectConceptId = string(data.subjectConceptId); const subjectLabel = string(data.subjectLabel);
  const predicateKind = string(data.predicateKind); const objectConceptId = string(data.objectConceptId);
  const objectLabel = string(data.objectLabel); const hop = integer(data.hop);
  return relationAssertionId === undefined || relationRevision === undefined || relationRevision < 1
    || subjectConceptId === undefined || subjectLabel === undefined || predicateKind === undefined
    || objectConceptId === undefined || objectLabel === undefined || hop === undefined || hop < 1
    ? undefined : { relationAssertionId, relationRevision, subjectConceptId, subjectLabel,
      predicateKind, objectConceptId, objectLabel, hop };
}

function source(value: unknown): ConceptRetrievalSource | undefined {
  const data = record(value);
  if (data === undefined) return undefined;
  const anchorId = string(data.anchorId); const conceptId = string(data.conceptId);
  const conceptLabel = string(data.conceptLabel); const paperId = string(data.paperId);
  const paperTitle = string(data.paperTitle);
  const sourceKind = oneOf(data.sourceKind, ["definition_claim", "term_usage"] as const);
  const sourceSubjectId = string(data.sourceSubjectId);
  const sourceSubjectRevision = integer(data.sourceSubjectRevision);
  const pageIndex = integer(data.pageIndex); const pageLabel = optionalString(data.pageLabel);
  const quoteExact = string(data.quoteExact); const contextText = string(data.contextText);
  const selected = boolean(data.selected);
  const omissionReason = data.omissionReason === undefined ? null
    : oneOf(data.omissionReason, ["source_limit", "duplicate_anchor"] as const);
  if (anchorId === undefined || conceptId === undefined || conceptLabel === undefined
    || paperId === undefined || paperTitle === undefined || sourceKind === undefined
    || sourceSubjectId === undefined || sourceSubjectRevision === undefined
    || sourceSubjectRevision < 1 || pageIndex === undefined || pageIndex < 0
    || pageLabel === undefined || quoteExact === undefined || contextText === undefined
    || selected === undefined || omissionReason === undefined
    || (selected && omissionReason !== null) || (!selected && omissionReason === null)) return undefined;
  return { anchorId, conceptId, conceptLabel, paperId, paperTitle, sourceKind,
    sourceSubjectId, sourceSubjectRevision, pageIndex, quoteExact, contextText, selected,
    ...(pageLabel === null ? {} : { pageLabel }),
    ...(omissionReason === null ? {} : { omissionReason }) };
}

export function decodeConceptRetrievalPreview(value: unknown): ConceptRetrievalPreview | undefined {
  const data = record(value);
  if (data === undefined) return undefined;
  const traceId = string(data.traceId);
  const semanticStatus = oneOf(data.semanticStatus, ["used", "unavailable"] as const);
  const embeddingSpaceId = optionalString(data.embeddingSpaceId);
  const indexGenerationId = optionalString(data.indexGenerationId);
  const seeds = list(data.seeds, seed); const edges = list(data.edges, edge);
  const sources = list(data.sources, source); const selectedSourceCount = integer(data.selectedSourceCount);
  const externalPaperCount = integer(data.externalPaperCount);
  const contextSha256 = string(data.contextSha256); const expiresAt = string(data.expiresAt);
  const invalidIndexPair = (embeddingSpaceId === null) !== (indexGenerationId === null);
  if (traceId === undefined || semanticStatus === undefined || embeddingSpaceId === undefined
    || indexGenerationId === undefined || invalidIndexPair || seeds === undefined || edges === undefined
    || sources === undefined || selectedSourceCount === undefined || selectedSourceCount < 0
    || externalPaperCount === undefined || externalPaperCount < 0 || contextSha256 === undefined
    || contextSha256.length !== 64 || expiresAt === undefined
    || selectedSourceCount !== sources.filter((item) => item.selected).length) return undefined;
  return { traceId, semanticStatus, seeds, edges, sources, selectedSourceCount,
    externalPaperCount, contextSha256, expiresAt,
    ...(embeddingSpaceId === null ? {} : { embeddingSpaceId }),
    ...(indexGenerationId === null ? {} : { indexGenerationId }) };
}
