import type { AiEvidenceItem } from "@research-assistant/core/ai";
import { DEFAULT_PORTRAIT_CATALOG, decodePortraitCatalog } from "@research-assistant/core/ai";
import {
  structuredResourceFields,
  type StructuredResource,
  type StructuredResourceDraft,
  type StructuredResourceWrite,
  type StructuredWriteResult,
  type StructuredStartupImport,
} from "@research-assistant/core/structured-resources";
import type { StructuredResourceGateway } from "../../../core/services/structuredResourceGateway";
import { getAiAssistanceGateway } from "../../../core/services/aiAssistanceGateway";
import { getResearchKnowledgeGateway } from "@research-assistant/literature-library-plugin/knowledgeGateway";
import { decodeStructuredState, encodeStructuredState } from "./mockStructuredStateCodec";
import { importMockStructuredPaper } from "./mockStructuredLegacy";
import { adoptMockStructuredTemplate, assertMockStructuredContent, mockPendingFields, reviewMockStructuredResource, synchronizeMockHumanBaseline } from "./mockStructuredReview";
import { importMockReadingProfiles, readMockProfileSelections, resolveMockReadingProfile, type MockProfileVersions } from "./mockStructuredProfiles";
import { createMockStructuredHbs, type MockStructuredHbsPort, type MockStructuredHbsState } from "./mockStructuredHbs";
import { flushStorage } from "../../../core/runtime/storagePersistence";

interface State extends MockStructuredHbsState {
  sources?: Record<string, readonly AiEvidenceItem[]>;
  receipts: Record<string, StructuredWriteResult>;
  reviewReceipts?: Record<string, StructuredResource>;
  historyTransitions?: Record<string, {before: StructuredResource; after: StructuredResource; protected: readonly string[]}>;
  historyReceipts?: Record<string, string>;
  imported: string[];
  profileVersions?: MockProfileVersions;
}

async function hash(text: string): Promise<string> {
  const bytes = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return Array.from(new Uint8Array(bytes), byte => byte.toString(16).padStart(2, "0")).join("");
}

/** Storage/provider adapter only; production review components render these records. */
export function createMockStructuredResources(storage: Storage, options: {
  readonly importContexts?: (contexts: StructuredStartupImport["contexts"]) => Promise<void>;
} = {}): StructuredResourceGateway & {readonly hbs: MockStructuredHbsPort} {
  const key = "mock-structured-resources:v1";
  const stored = storage.getItem(key);
  const state: State = stored ? decodeStructuredState(stored) : {
    resources: [], versions: {}, baselineReviewed: {}, receipts: {}, imported: [],
  };
  const save = (): void => storage.setItem(key, encodeStructuredState(state));

  // Persist the resource, history transition and receipt together before notifying views.
  function commitReview(nextState: State, resource: StructuredResource): void {
    storage.setItem(key, encodeStructuredState(nextState));
    Object.assign(state, nextState);
    window.dispatchEvent(new CustomEvent("research-assistant:structured-resource-changed", {
      detail: {resourceId: resource.id, paperId: resource.paperId ?? null},
    }));
  }
  function withResource(resource: StructuredResource): Pick<State, "resources" | "versions"> {
    return {resources: [...state.resources.filter(value => value.id !== resource.id), structuredClone(resource)],
      versions: {...state.versions, [`${resource.id}:${resource.revision}`]: structuredClone(resource)}};
  }

  function get(id: string): StructuredResource {
    const resource = state.resources.find(value => value.id === id);
    if (!resource) throw new Error("结构化资源不存在");
    return structuredClone(resource);
  }

  function put(resource: StructuredResource): void {
    state.resources = state.resources.filter(value => value.id !== resource.id);
    state.resources.push(structuredClone(resource));
    if (resource.revision > 0) state.versions[`${resource.id}:${resource.revision}`] = structuredClone(resource);
    save();
    window.dispatchEvent(new CustomEvent("research-assistant:structured-resource-changed", {
      detail: {resourceId: resource.id, paperId: resource.paperId ?? null},
    }));
  }

  function ensurePaper(paperId: string): void {
    for (const kind of ["portrait", "structuredReading", "readingTemplate"] as const) {
      if (state.resources.some(value => value.paperId === paperId && value.kind === kind)) continue;
      const id = crypto.randomUUID();
      put({
        id, kind, paperId, sourceUri: `source://library/${paperId}/${id}`,
        title: kind === "portrait" ? "文献画像" : kind === "structuredReading" ? "结构化阅读记录" : "本文阅读模板",
        revision: 0, content: null, reviewBaseline: null, pendingFieldIds: [], reviewedFieldIds: [],
      });
    }
    const owned = state.resources.find(value => value.paperId === paperId && value.kind === "readingTemplate")!;
    const reading = state.resources.find(value => value.paperId === paperId && value.kind === "structuredReading")!;
    const bound = reading.templateRef
      ? state.versions[`${reading.templateRef.resourceId}:${reading.templateRef.revision}`] : undefined;
    // Repair the earlier preview representation without overwriting a published object.
    if (owned.revision === 0 && !owned.draft && bound && !bound.paperId &&
      bound.title === "本文阅读模板" && bound.content) {
      put({...owned, revision: 1, content: bound.content, reviewBaseline: bound.content});
      put({...reading, revision: reading.revision + 1, templateRef: {resourceId: owned.id, revision: 1}});
    }
  }

  function ensureTemplates(): void {
    if (state.resources.some(value => value.kind === "readingTemplate" && !value.paperId)) return;
    for (const template of DEFAULT_PORTRAIT_CATALOG.templates) {
      const id = crypto.randomUUID();
      const content = {
        kind: "reading_template_v1", schemaVersion: 1, name: template.label,
        fields: template.fields.map(({claimIds: _, ...field}) => field),
      };
      put({
        id, kind: "readingTemplate", title: template.label,
        sourceUri: `source://agents/skills/edit-structured-resources/assets/reading/${id}.json`,
        revision: 1, content, reviewBaseline: content, pendingFieldIds: [], reviewedFieldIds: [],
      });
    }
  }

  function importCatalog(raw: unknown): void {
    for (const template of decodePortraitCatalog(raw).templates) {
      const marker = `catalog:${template.id}`;
      if (state.imported.includes(marker)) continue;
      const content = {kind: "reading_template_v1", schemaVersion: 1, name: template.label,
        fields: template.fields.map(({claimIds: _, ...field}) => field)};
      const existing = state.resources.find(value => value.kind === "readingTemplate" && !value.paperId &&
        JSON.stringify(value.content) === JSON.stringify(content));
      if (!existing) {
        const id = crypto.randomUUID();
        put({id, kind: "readingTemplate", title: template.label,
          sourceUri: `source://agents/skills/edit-structured-resources/assets/reading/${id}.json`, revision: 1, content,
          reviewBaseline: content, pendingFieldIds: [], reviewedFieldIds: []});
      }
      state.imported.push(marker);
    }
    save();
  }

  function remember(requestId: string, result: StructuredWriteResult): StructuredWriteResult {
    state.receipts[requestId] = structuredClone(result);
    save();
    return result;
  }

  async function write(input: StructuredResourceWrite): Promise<StructuredWriteResult> {
    const prior = state.receipts[input.requestId];
    if (prior) return structuredClone(prior);
    const resource = get(input.resourceId);
    const errors: StructuredResourceDraft["errors"][number][] = [];
    let content: unknown;
    try {
      content = JSON.parse(input.content);
      assertMockStructuredContent(resource.kind, content);
      if (resource.kind === "structuredReading" && !("retainedFields" in (content as object))) {
        const retained = (resource.content as {retainedFields?: unknown} | null)?.retainedFields;
        if (retained !== undefined) content = {...content as object, retainedFields: retained};
      }
    } catch (reason) {
      errors.push({path: "$", code: "invalid_input", message: reason instanceof Error ? reason.message : "JSON 格式无效"});
    }
    const contentHash = await hash(input.content);
    const draft: StructuredResourceDraft = {
      id: resource.draft?.id ?? crypto.randomUUID(), resourceId: resource.id,
      baseRevision: input.expectedRevision, revision: (resource.draft?.revision ?? 0) + 1,
      content: input.content, contentHash, errors,
      ...(input.conversationId ? {creatorConversationId: input.conversationId} : {}),
    };
    const conflict = resource.revision !== input.expectedRevision || input.expectedDraftRevision !== undefined &&
      input.expectedDraftRevision !== resource.draft?.revision;
    if (conflict) errors.push({path: "$", code: "conflict", message: "正式内容或草稿已更新，请读取后重试。"});
    if (conflict || errors.length) {
      const updated = {...resource, draft};
      put(updated);
      return remember(input.requestId, {
        status: conflict ? "conflict" : "invalid",
        resource: updated, draft, errors,
      });
    }
    const baseline = input.editorKind === "human" ? synchronizeMockHumanBaseline(resource, content)
      : resource.pendingFieldIds.length ? resource.reviewBaseline : resource.content;
    if (!resource.pendingFieldIds.length) state.baselineReviewed[resource.id] = resource.reviewedFieldIds;
    const before = structuredResourceFields(resource.content);
    const after = structuredResourceFields(content);
    const reviewedFieldIds = resource.reviewedFieldIds.filter(id =>
      JSON.stringify(before.find(field => field.id === id)?.value) ===
      JSON.stringify(after.find(field => field.id === id)?.value));
    const {draft: _, ...withoutDraft} = resource;
    const updated = {
      ...withoutDraft, revision: resource.revision + 1, content, reviewBaseline: baseline,
      ...(resource.kind === "readingTemplate" ? {title:(content as {name:string}).name} : {}),
      pendingFieldIds: mockPendingFields(content, baseline), reviewedFieldIds,
    };
    put(updated);
    if (input.editorKind !== "agent") {
      state.historyTransitions ??= {};
      state.historyTransitions[`${resource.id}:${updated.revision}`] = {
        before: resource, after: updated, protected: state.baselineReviewed[resource.id] ?? [],
      };
    }
    return remember(input.requestId, {
      status: "applied", resource: updated, errors: [],
      receipt: {requestId: input.requestId, resourceId: resource.id, revision: updated.revision, contentHash},
    });
  }

  ensureTemplates();
  state.profileVersions ??= {};
  importMockReadingProfiles(state.profileVersions, put);
  save();
  const hbs = createMockStructuredHbs({state: () => state, flush: () => flushStorage(storage),
    persist: async next => {
      storage.setItem(key, encodeStructuredState(next));
      Object.assign(state, next);
      await flushStorage(storage);
    }, notify: resource => window.dispatchEvent(new CustomEvent("research-assistant:structured-resource-changed", {
        detail: {resourceId: resource.id, paperId: resource.paperId ?? null},
      }))});
  const gateway: StructuredResourceGateway & {readonly hbs: MockStructuredHbsPort} = {
    hbs,
    subscribeChanges: async changed => {
      const listener = (event: Event): void => changed(
        (event as CustomEvent<{resourceId: string; paperId: string | null}>).detail,
      );
      window.addEventListener("research-assistant:structured-resource-changed", listener);
      return () => window.removeEventListener("research-assistant:structured-resource-changed", listener);
    },
    createTemplate: async title => {
      const id = crypto.randomUUID();
      const resource: StructuredResource = {
        id, kind: "readingTemplate", title, sourceUri: `source://agents/skills/edit-structured-resources/assets/reading/${id}.json`,
        revision: 0, content: null, reviewBaseline: null, pendingFieldIds: [], reviewedFieldIds: [],
      };
      put(resource);
      return resource;
    },
    list: async paperId => {
      if (paperId) ensurePaper(paperId);
      return structuredClone(state.resources.filter(value => !paperId || value.paperId === paperId));
    },
    getSources: async id => {
      if (state.sources?.[id]) return structuredClone(state.sources[id]);
      const resource = get(id);
      if (!resource.paperId) return [];
      const candidates = await getAiAssistanceGateway().listCandidates();
      const content = JSON.stringify(resource.content);
      const candidate = candidates.find(value =>
        value.evidenceBundle.items.every(source => source.paperId === resource.paperId) &&
        value.revisions.some(revision => JSON.stringify(revision.payload) === content));
      const sources = candidate?.evidenceBundle.items ?? [];
      if (sources.length) {
        state.sources ??= {};
        state.sources[id] = structuredClone(sources);
        save();
      }
      return structuredClone(sources);
    },
    get: async id => get(id),
    getVersion: async (id, revision) => {
      const version = state.versions[`${id}:${revision}`];
      if (!version) throw new Error("资源版本不存在");
      return structuredClone(version);
    },
    write,
    review: async request => {
      const prior = state.reviewReceipts?.[request.requestId];
      if (prior) return structuredClone(prior);
      const current = get(request.resourceId);
      if (current.revision !== request.expectedRevision) throw new Error("内容已更新，请重新审阅。");
      const protectedBefore = state.baselineReviewed[current.id] ?? [];
      let next: StructuredResource;
      if (request.action === "undoGeneration") {
        const clean = Object.values(state.versions).filter(value => value.id === current.id &&
          value.revision < current.revision && value.pendingFieldIds.length === 0)
          .sort((left, right) => right.revision - left.revision)[0];
        next = {...current, content: clean?.content ?? null, reviewBaseline: clean?.content ?? null,
          pendingFieldIds: [], reviewedFieldIds: clean?.reviewedFieldIds ?? [], revision: current.revision + 1};
      } else {
        next = reviewMockStructuredResource(current, request, state.baselineReviewed[current.id] ?? []);
      }
      commitReview({...state, ...withResource(next),
        baselineReviewed: request.action === "clear" ? {...state.baselineReviewed, [current.id]: []} : state.baselineReviewed,
        historyTransitions: {...state.historyTransitions, [`${current.id}:${next.revision}`]: {
          before: current, after: structuredClone(next), protected: protectedBefore,
        }},
        reviewReceipts: {...state.reviewReceipts, [request.requestId]: structuredClone(next)},
      }, next);
      return next;
    },
    restoreHistory: async request => {
      const fingerprint = JSON.stringify([request.resourceId,request.expectedRevision,request.operationRevision,request.direction,request.requestId]);
      if (state.historyReceipts?.[request.requestId]) {
        if (state.historyReceipts[request.requestId] !== fingerprint) throw new Error("操作身份已用于不同请求");
        return get(request.resourceId);
      }
      const current = get(request.resourceId);
      if (current.revision !== request.expectedRevision) throw new Error("内容已更新，请重新读取。");
      const transition = state.historyTransitions?.[`${current.id}:${request.operationRevision}`];
      if (!transition) throw new Error("找不到对应的人工修改或审阅回执");
      const side = request.direction === "undo" ? "after" : "before";
      const snapshot = (value: StructuredResource): string => JSON.stringify([
        value.content,value.reviewBaseline,value.reviewedFieldIds,value.templateRef ?? null,
      ]);
      if (snapshot(current) !== snapshot(transition[side])) throw new Error("内容已更新，不能覆盖其他修改。");
      const target = request.direction === "undo" ? transition.before : transition.after;
      const restored = {...current, content: target.content, reviewBaseline: target.reviewBaseline,
        reviewedFieldIds: target.reviewedFieldIds, pendingFieldIds: target.pendingFieldIds,
        revision: current.revision + 1};
      if (target.templateRef) Object.assign(restored,{templateRef:target.templateRef});
      else delete (restored as {templateRef?: unknown}).templateRef;
      commitReview({...state, ...withResource(restored),
        baselineReviewed: {...state.baselineReviewed, [current.id]: transition.protected},
        historyReceipts: {...state.historyReceipts, [request.requestId]: fingerprint},
      }, restored);
      return restored;
    },
    bindTemplate: async request => {
      ensurePaper(request.paperId);
      const template = state.versions[`${request.templateId}:${request.templateRevision}`];
      const reading = state.resources.find(value => value.paperId === request.paperId && value.kind === "structuredReading")!;
      if (!template || reading.revision !== request.expectedRevision) throw new Error("模板或阅读版本已变化");
      if(template.paperId && template.paperId!==request.paperId)throw new Error("不能采用其他文献的本文模板");
      if ((reading.templateRef || reading.content) && !request.confirmed) throw new Error("切换模板需要确认");
      const next = adoptMockStructuredTemplate(reading, template);
      put(next);
      return next;
    },
    importLegacy: async request => {
      ensureTemplates();
      if (request.catalog !== undefined) importCatalog(request.catalog);
      if (!request.paperId) return structuredClone(state.resources.filter(value =>
        value.kind === "readingTemplate" && !value.paperId));
      ensurePaper(request.paperId);
      if (!state.imported.includes(request.paperId)) {
        await importMockStructuredPaper({...request, paperId: request.paperId}, state.resources, put, get, {
          selection: readMockProfileSelections(storage).find(value => value.paperId === request.paperId)?.profiles ?? [],
          resolveProfile: reference => resolveMockReadingProfile(state.profileVersions!, reference, (id, revision) => {
            const version = state.versions[`${id}:${revision}`];
            if (!version) throw new Error("旧阅读框架的准确资源版本不存在");
            return structuredClone(version);
          }),
          proposeTemplate: async resource => {
            await write(resource);
          },
        });
        state.imported.push(request.paperId);
        save();
      }
      return structuredClone(state.resources.filter(value => value.paperId === request.paperId));
    },
    importStartup: async draft => {
      if (draft.catalog !== undefined) importCatalog(draft.catalog);
      const [candidates, units] = await Promise.all([
        getAiAssistanceGateway().listCandidates(), getResearchKnowledgeGateway().listReadingUnits(),
      ]);
      const paperIds = [...new Set([
        ...candidates.filter(candidate => ["paper_portrait_v1", "paper_overview_unit_v1"].includes(candidate.candidateKind))
          .flatMap(candidate => candidate.evidenceBundle.items.map(source => source.paperId)),
        ...units.map(unit => unit.paperId).filter((id): id is string => !!id),
        ...draft.presentations.map(value => value.paperId),
        ...readMockProfileSelections(storage).filter(value => value.profiles.length > 0).map(value => value.paperId),
      ])];
      let importedPaperCount = 0;
      for (const paperId of paperIds) {
        const presentation = draft.presentations.find(value => value.paperId === paperId)?.presentation;
        if (!state.imported.includes(paperId)) importedPaperCount++;
        await gateway.importLegacy({paperId, requestId: `startup:${paperId}`,
          ...(presentation ? {presentation} : {})});
      }
      await options.importContexts?.(draft.contexts);
      return {paperIds, importedPaperCount, importedTemplateCount: state.resources.filter(value =>
        value.kind === "readingTemplate" && !value.paperId && value.revision > 0).length};
    },
  };
  return gateway;
}
