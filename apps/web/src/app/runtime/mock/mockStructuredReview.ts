import {
  structuredResourceDiffFields,
  structuredResourceFields,
  type StructuredResource,
  type StructuredResourceKind,
  type StructuredReviewRequest,
} from "@research-assistant/core/structured-resources";

export function assertMockStructuredContent(kind: StructuredResourceKind, content: unknown): void {
  if (!content || typeof content !== "object" || Array.isArray(content)) throw new Error("需要 JSON 对象");
  const body = content as Record<string, unknown>;
  if (body.schemaVersion !== 1) throw new Error("不支持此格式版本");
  const expected = kind === "portrait" ? "paper_portrait_v1"
    : kind === "structuredReading" ? "paper_overview_unit_v1" : "reading_template_v1";
  if (body.kind !== expected) throw new Error("资源格式与类型不匹配");
  if (kind === "readingTemplate" && (
    typeof body.name !== "string" || !body.name.trim() ||
    !Array.isArray(body.fields) || !body.fields.length
  )) throw new Error("模板需要名称和字段");
  if (kind === "portrait") {
    if (!Array.isArray(body.claims)) throw new Error("画像需要分类判断");
    const dimensions = ["genre", "mode", "paradigm"];
    const custom = Array.isArray(body.customDimensions) ? body.customDimensions : [];
    dimensions.push(...custom.map(value => (value as {id: string}).id));
    for (const dimension of dimensions) {
      if (!body.claims.some(value => (value as {dimension: string}).dimension === dimension)) {
        throw new Error("画像必须保留全部分类维度");
      }
    }
  }
  if (kind === "structuredReading" && !Array.isArray(body.fields)) throw new Error("阅读需要字段列表");
  const fields = structuredResourceFields(content);
  if (new Set(fields.map(field => field.id)).size !== fields.length) throw new Error("字段身份重复");
}

export function mockPendingFields(content: unknown, baseline: unknown): string[] {
  const current = structuredResourceDiffFields(content);
  const before = structuredResourceDiffFields(baseline);
  return [...new Set([...current, ...before].map(field => field.id))].filter(id =>
    JSON.stringify(current.find(field => field.id === id)?.value ?? null) !==
    JSON.stringify(before.find(field => field.id === id)?.value ?? null));
}

function replaceActiveFields(
  target: unknown,
  source: unknown,
  selected: ReadonlySet<string>,
  baseline: boolean,
): unknown {
  if (!target) {
    if (!source) return null;
    const body = source as Record<string, unknown>;
    target = {[Array.isArray(body.claims) ? "claims" : "fields"]: []};
  }
  let body = target as Record<string, unknown>;
  const key = Array.isArray(body.claims) ? "claims" : "fields";
  if (selected.has("$metadata")) {
    const metadata = source && typeof source === "object" ? Object.fromEntries(
      Object.entries(source).filter(([name]) => !["fields", "claims", "retainedFields"].includes(name)),
    ) : {};
    body = {...metadata, [key]: body[key],
      ...(body.retainedFields === undefined ? {} : {retainedFields: body.retainedFields})};
  }
  const sourceFields = structuredResourceFields(source);
  const entries = [...structuredResourceFields(body).map(field => field.value)];
  const identity = (value: unknown): string | undefined => {
    const entry = value as {field?: string; id?: string};
    return entry.field ?? entry.id;
  };
  const order = sourceFields.map(field => field.id);
  if (selected.has("$order")) {
    if (baseline) for (const id of order) {
      if (!entries.some(entry => identity(entry) === id)) entries.push({[key === "claims" ? "id" : "field"]: id});
    }
    const position = (value: unknown): number => {
      const index = order.indexOf(identity(value) ?? "");
      return index < 0 ? order.length : index;
    };
    entries.sort((left, right) => position(left) - position(right));
  }
  for (const id of [...selected].filter(value => !value.startsWith("$"))) {
    const position = entries.findIndex(entry => identity(entry) === id);
    const replacement = sourceFields.find(field => field.id === id);
    if (position >= 0 && replacement) entries[position] = replacement.value;
    else if (position >= 0) entries.splice(position, 1);
    else if (replacement) {
      const following = order.slice(order.indexOf(id) + 1);
      const nextPosition = following.map(nextId => entries.findIndex(entry => identity(entry) === nextId))
        .find(index => index >= 0);
      entries.splice(nextPosition ?? entries.length, 0, replacement.value);
    }
  }
  return {...body, [key]: entries};
}

function replaceFields(target: unknown, source: unknown, selected: ReadonlySet<string>, baseline: boolean): unknown {
  const result = replaceActiveFields(target, source, selected, baseline);
  const ids = [...selected].filter(id => id.startsWith("$retained:")).map(id => id.slice(10));
  if (!ids.length) return result;
  const targetRetained = (target as {retainedFields?: unknown[]} | null)?.retainedFields ?? [];
  const sourceRetained = (source as {retainedFields?: unknown[]} | null)?.retainedFields ?? [];
  const replaced = replaceActiveFields({fields: targetRetained}, {fields: sourceRetained}, new Set(ids), baseline) as {fields: unknown[]};
  return {...result as object, retainedFields: replaced.fields};
}

/** Human edits in already-clean fields advance B, matching native per-field synchronization. */
export function synchronizeMockHumanBaseline(resource: StructuredResource, content: unknown): unknown {
  if (!resource.pendingFieldIds.length) return content;
  const changed = mockPendingFields(content, resource.content)
    .filter(id => !resource.pendingFieldIds.includes(id));
  return replaceFields(resource.reviewBaseline, content, new Set(changed), true);
}

export function reviewMockStructuredResource(
  resource: StructuredResource,
  request: StructuredReviewRequest,
  baselineReviewed: readonly string[],
): StructuredResource {
  if (resource.revision !== request.expectedRevision) throw new Error("内容已更新，请重新审阅。");
  if (request.action === "clear") {
    if (resource.kind !== "structuredReading" || request.fieldIds !== undefined || request.scope !== undefined)
      throw new Error("清空操作只适用于整份结构化阅读记录");
    return {...resource, revision: resource.revision + 1, content: null, reviewBaseline: null,
      pendingFieldIds: [], reviewedFieldIds: []};
  }
  const verifying = request.action === "verify" || request.action === "resetVerification";
  const available = verifying ? structuredResourceFields(resource.content).map(field => field.id)
    : resource.pendingFieldIds;
  if (request.fieldIds?.some(id => !available.includes(id))) throw new Error("审阅字段已变化，请重新读取");
  const selected = request.fieldIds ?? available;
  const fields = new Set(selected.filter(id => request.scope !== "unreviewed" ||
    !baselineReviewed.includes(id) && !resource.reviewedFieldIds.includes(id)));
  if (verifying) return {
    ...resource,
    revision: resource.revision + 1,
    reviewedFieldIds: request.action === "verify"
      ? [...new Set([...resource.reviewedFieldIds, ...fields])]
      : resource.reviewedFieldIds.filter(id => !fields.has(id)),
  };
  let next: StructuredResource;
  if (request.action === "keep") {
    next = {...resource, revision: resource.revision + 1,
      reviewBaseline: replaceFields(resource.reviewBaseline, resource.content, fields, true)};
  } else {
    const content = resource.pendingFieldIds.every(id => fields.has(id))
      ? resource.reviewBaseline : replaceFields(resource.content, resource.reviewBaseline, fields, false);
    // A partial Undo may not remove schema identity or a required portrait dimension.
    // Undo all can restore the stable empty resource.
    if (content !== null) {
      try { assertMockStructuredContent(resource.kind, content); }
      catch { throw new Error("此撤销会破坏资源格式，请撤销全部修改或同时选择相关字段。"); }
    }
    next = {
      ...resource, content, revision: resource.revision + 1,
      reviewedFieldIds: [...new Set([
        ...resource.reviewedFieldIds.filter(id => !fields.has(id)),
        ...baselineReviewed.filter(id => fields.has(id)),
      ])],
    };
  }
  return {...next, pendingFieldIds: mockPendingFields(next.content, next.reviewBaseline),
    ...(next.kind === "readingTemplate" && next.content ? {title:(next.content as {name:string}).name} : {})};
}

/** Switching templates preserves excluded fields and restores them when their IDs return. */
export function adoptMockStructuredTemplate(reading: StructuredResource, template: StructuredResource): StructuredResource {
  if (!reading.content) return {...reading, revision: reading.revision + 1,
    templateRef: {resourceId: template.id, revision: template.revision}};
  const body = reading.content as Record<string, unknown>;
  const definitions = (template.content as {fields: readonly {id: string}[]}).fields;
  const active = structuredResourceFields(body);
  const oldRetained = structuredResourceFields({fields: body.retainedFields ?? []});
  const ids = new Set(definitions.map(field => field.id));
  const fields = definitions.map(definition => active.find(field => field.id === definition.id)?.value ??
    oldRetained.find(field => field.id === definition.id)?.value ??
    {field: definition.id, status: "not_found", statements: []});
  const retainedFields = [...oldRetained, ...active.filter(field => !ids.has(field.id))]
    .filter((field, index, rows) => !ids.has(field.id) && rows.findIndex(value => value.id === field.id) === index)
    .map(field => field.value);
  const content = {...body, fields, retainedFields, paperTemplate: {
    templateResourceId: template.id, templateResourceRevision: template.revision,
    fields: definitions.map(field => ({...field, claimIds: []})),
  }};
  const baseline = reading.pendingFieldIds.length ? reading.reviewBaseline : reading.content;
  return {...reading, revision: reading.revision + 1, content, reviewBaseline: baseline,
    pendingFieldIds: mockPendingFields(content, baseline),
    templateRef: {resourceId: template.id, revision: template.revision}};
}
