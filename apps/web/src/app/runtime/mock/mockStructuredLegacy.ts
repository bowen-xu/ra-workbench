import { selectPaperPortraitTemplate } from "@research-assistant/core/ai";
import type { ReadingProfileRef } from "@research-assistant/core/knowledge";
import type { StructuredLegacyImport, StructuredResource, StructuredResourceWrite } from "@research-assistant/core/structured-resources";
import { getAiAssistanceGateway } from "../../../core/services/aiAssistanceGateway";
import { getResearchKnowledgeGateway } from "@research-assistant/literature-library-plugin/knowledgeGateway";
import { adoptMockStructuredTemplate } from "./mockStructuredReview";
import { importMockLegacyTemplate } from "./mockStructuredLegacyTemplate";

/** Imports the user's selected mixed candidate view, including manual field bodies. */
export async function importMockStructuredPaper(
  request: StructuredLegacyImport & {paperId: string},
  resources: readonly StructuredResource[],
  put: (resource: StructuredResource) => void,
  get: (id: string) => StructuredResource,
  profiles: {
    readonly selection: readonly ReadingProfileRef[];
    readonly resolveProfile: (reference: ReadingProfileRef) => StructuredResource;
    readonly proposeTemplate: (request: StructuredResourceWrite) => Promise<void>;
  },
): Promise<void> {
  const candidates = await getAiAssistanceGateway().listCandidates();
  const portraitSelection = selectPaperPortraitTemplate(candidates, request.paperId);
  const portrait = portraitSelection?.candidate.revisions
    .find(value => value.revision === portraitSelection.revision)?.payload;
  const owned = candidates.filter(candidate =>
    candidate.candidateKind === "paper_overview_unit_v1" && candidate.reviewState !== "rejected" &&
    candidate.evidenceBundle.items.every(source => source.paperId === request.paperId),
  ).sort((left, right) => right.createdAt.localeCompare(left.createdAt));
  const latest = owned[0];
  const readingPayload = latest?.revisions.find(value => value.revision === latest.currentRevision)?.payload;
  const units = await getResearchKnowledgeGateway().listReadingUnits(undefined, request.paperId);
  const paperResources = resources.filter(value => value.paperId === request.paperId);
  const template = paperResources.find(value => value.kind === "readingTemplate")!;
  const resolvedTemplate = importMockLegacyTemplate(
    request, owned, profiles.selection, units.length > 0, template, profiles.resolveProfile, put,
  );
  const projection = request.presentation?.fields ?? (readingPayload?.kind === "paper_overview_unit_v1"
    ? readingPayload.fields.map(field => ({candidateId: latest!.id, fieldId: field.field})) : []);
  const fields: unknown[] = projection.flatMap(pointer => {
    const candidate = owned.find(value => value.id === pointer.candidateId);
    const payload = candidate?.revisions.find(value => value.revision === candidate.currentRevision)?.payload;
    const field = payload?.kind === "paper_overview_unit_v1"
      ? payload.fields.find(value => value.field === pointer.fieldId) : undefined;
    if (!field) return [];
    const confirmation = candidate?.overviewFieldConfirmations.find(value =>
      value.fieldKind === field.field && (value.active || value.reviewResetAt));
    const unit = units.find(value => value.id === confirmation?.targetReadingUnitId &&
      !request.presentation?.hiddenUnitIds.includes(value.id));
    if (!unit) return [field];
    return [{...field, statements: [{
      ...field.statements[0],
      id: field.statements[0]?.id ?? `${field.field}_manual`,
      text: unit.body,
      epistemicRole: field.statements[0]?.epistemicRole ?? "uncertain",
      evidenceItemIds: field.statements.flatMap(statement => statement.evidenceItemIds),
    }]}];
  });
  for (const unit of [...units].sort((left, right) => left.updatedAt.localeCompare(right.updatedAt))) {
    if (request.presentation?.hiddenUnitIds.includes(unit.id)) continue;
    const fieldId = unit.templateField?.fieldId ?? unit.unitKind;
    const position = fields.findIndex(value => (value as {field: string}).field === fieldId);
    const existing = position < 0 ? undefined : fields[position] as {statements?: readonly {evidenceItemIds?: readonly string[]}[]};
    const field = {field: fieldId, status: "filled", statements: [{id: `legacy-${unit.id}`,
      text: unit.body, epistemicRole: "working_note",
      evidenceItemIds: existing?.statements?.flatMap(statement => statement.evidenceItemIds ?? []) ?? []}]};
    if (position < 0) fields.push(field);
    else fields[position] = field;
  }
  const importedReading = fields.length === 0 ? undefined : readingPayload?.kind === "paper_overview_unit_v1"
    ? {...readingPayload, fields} : {kind: "paper_overview_unit_v1", schemaVersion: 1,
      profileId: "generic_overview_v1", fields};
  for (const resource of paperResources.filter(value => value.revision === 0)) {
    const content = resource.kind === "portrait" ? portrait
      : resource.kind === "structuredReading" ? importedReading : undefined;
    if (content) {
      let imported: StructuredResource = {...resource, revision: 1, content, reviewBaseline: content};
      if (resource.kind === "structuredReading" && resolvedTemplate) {
        const adopted = adoptMockStructuredTemplate(imported, resolvedTemplate);
        const {profileRefs: _, ...body} = adopted.content as Record<string, unknown>;
        const formal = {...body, profileId: "paper_template_v1"};
        imported = {...adopted, revision: 1, content: formal, reviewBaseline: formal, pendingFieldIds: []};
      }
      put(imported);
    } else if (resource.kind === "structuredReading" && resolvedTemplate) {
      put({...resource, revision: 1, templateRef: {resourceId: resolvedTemplate.id, revision: resolvedTemplate.revision}});
    }
  }
  if (portrait?.kind !== "paper_portrait_v1" || resolvedTemplate || template.revision > 0 || template.draft) return;
  // A portrait without an adopted reading snapshot only recommends a template.
  const content = {
    kind: "reading_template_v1", schemaVersion: 1, name: "本文阅读模板",
    fields: portrait.template.map(({claimIds: _, ...field}) => field),
  };
  await profiles.proposeTemplate({resourceId: template.id, expectedRevision: get(template.id).revision,
    requestId: `legacy-template-proposal:${request.paperId}`, content: JSON.stringify(content), editorKind: "agent"});
}
