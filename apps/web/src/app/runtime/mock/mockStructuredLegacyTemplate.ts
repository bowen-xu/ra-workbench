import type { AiCandidate, PaperOverviewUnitCandidateV1 } from "@research-assistant/core/ai";
import type { ReadingProfileRef } from "@research-assistant/core/knowledge";
import type { StructuredLegacyImport, StructuredResource } from "@research-assistant/core/structured-resources";

type Definition = {readonly id: string; readonly title: string; readonly question: string; readonly color?: string};
const fieldsOf = (template: StructuredResource): readonly Definition[] =>
  (template.content as {fields: readonly Definition[]}).fields;
const references = (payload: PaperOverviewUnitCandidateV1): readonly ReadingProfileRef[] =>
  payload.profileId === "generic_overview_v1"
    ? [{profileId: "generic_overview", profileVersion: 1}] : payload.profileRefs ?? [];

/** Reconstruct the precise displayed field composition, before looking at later portrait suggestions. */
export function importMockLegacyTemplate(
  request: StructuredLegacyImport & {paperId: string},
  candidates: readonly AiCandidate[],
  selection: readonly ReadingProfileRef[],
  hasManualBody: boolean,
  target: StructuredResource,
  resolveProfile: (reference: ReadingProfileRef) => StructuredResource,
  put: (resource: StructuredResource) => void,
): StructuredResource | undefined {
  const latest = candidates[0];
  const current = (candidate: AiCandidate | undefined): PaperOverviewUnitCandidateV1 | undefined => {
    const payload = candidate?.revisions.find(value => value.revision === candidate.currentRevision)?.payload;
    return payload?.kind === "paper_overview_unit_v1" ? payload : undefined;
  };
  const projection = request.presentation?.fields ?? current(latest)?.fields.map(field =>
    ({candidateId: latest!.id, fieldId: field.field})) ?? [];
  const selected = projection.flatMap(pointer => {
    const payload = current(candidates.find(candidate => candidate.id === pointer.candidateId));
    return payload ? [{payload, fieldId: pointer.fieldId}] : [];
  });
  const custom = selected.some(({payload}) => payload.paperTemplate !== undefined);
  const refs = selected.length ? selected.flatMap(({payload}) => references(payload)) : [...selection];
  if (!selected.length && !refs.length && hasManualBody) refs.push({profileId: "generic_overview", profileVersion: 1});
  const uniqueRefs = refs.filter((reference, index) => refs.findIndex(value =>
    value.profileId === reference.profileId && value.profileVersion === reference.profileVersion) === index);
  if (!custom && !uniqueRefs.length) return undefined;
  if (!custom && uniqueRefs.length === 1) return resolveProfile(uniqueRefs[0]!);

  const definitions: Definition[] = [];
  const add = (field: Definition): void => {
    if (!definitions.some(value => value.id === field.id)) definitions.push(field);
  };
  for (const {payload, fieldId} of selected) {
    const customField = payload.paperTemplate?.fields.find(field => field.id === fieldId);
    const field = customField ?? references(payload).flatMap(reference => fieldsOf(resolveProfile(reference)))
      .find(value => value.id === fieldId);
    if (!field) throw new Error(`旧字段 ${fieldId} 没有对应冻结框架版本中的定义`);
    const {id, title, question, color} = field;
    add({id, title, question, ...(color === undefined ? {} : {color})});
  }
  if (!selected.length) for (const reference of uniqueRefs) fieldsOf(resolveProfile(reference)).forEach(add);
  if (!definitions.length) return undefined;
  if (target.revision > 0) return target;
  const content = {kind: "reading_template_v1", schemaVersion: 1, name: "本文阅读模板", fields: definitions};
  const imported = {...target, revision: 1, content, reviewBaseline: content};
  put(imported);
  return imported;
}
