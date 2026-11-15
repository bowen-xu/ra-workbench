import type { ReadingProfile, ReadingProfileRef, PaperReadingProfileSelection } from "@research-assistant/core/knowledge";
import { type StructuredResource } from "@research-assistant/core/structured-resources";
import { browserReadingProfiles } from "../../../../../../plugins/literature_library/src/services/browserReadingProfiles";
import { decodeList, decodePaperReadingProfileSelection } from "../../../../../../plugins/literature_library/src/services/knowledgeDecoders";

export interface MockProfileVersion {
  readonly resourceId: string;
  readonly revision: number;
  readonly source: ReadingProfile;
}

export type MockProfileVersions = Record<string, MockProfileVersion>;

const profileKey = (reference: ReadingProfileRef): string =>
  `reading-profile:${reference.profileId}@${reference.profileVersion}`;

/** These definitions mirror the deployed reviewed profile versions, not a label lookup. */
export function legacyProfileField(id: string): {id: string; title: string; question: string} {
  const definitions: Readonly<Record<string, readonly [string, string]>> = {
    research_question: ["研究问题", "论文试图回答的研究问题、目标与需求是什么？"],
    approach: ["方法／论证路径", "论文采用何种方法、系统设计或论证路径来回答研究问题？"],
    key_result: ["关键结果", "论文报告哪些主要结果及其原文证据？"],
    conclusion: ["结论", "作者据现有证据提出哪些结论及其适用范围？"],
    limitation: ["局限", "论文承认或原文证据明确支持哪些局限？"],
    open_question: ["开放问题", "论文提出哪些尚待解决的问题或后续研究方向？"],
    argument: ["核心论证", "论文的核心论题、前提、推论及论证链条是什么？"],
    objection: ["异议／反例", "论文讨论哪些异议、反例及其针对的主张？"],
    response: ["回应", "论文如何回应这些异议或反例？"],
    other: ["其他", "需要保留的其他原文信息与人工研究记录是什么？"],
  };
  const definition = definitions[id];
  if (!definition) throw new Error(`旧阅读框架含未知字段 ${id}`);
  return {id, title: definition[0], question: definition[1]};
}

/** Keep an explicit source-version mapping even when two profiles have identical fields. */
export function importMockReadingProfiles(
  versions: MockProfileVersions,
  put: (resource: StructuredResource) => void,
): void {
  for (const profile of browserReadingProfiles) {
    const key = profileKey(profile);
    if (versions[key]) continue;
    const previous = Object.values(versions).filter(value => value.source.profileId === profile.profileId)
      .sort((left, right) => right.source.profileVersion - left.source.profileVersion)[0];
    const id = previous?.resourceId ?? crypto.randomUUID();
    const revision = (previous?.revision ?? 0) + 1;
    const content = {kind: "reading_template_v1", schemaVersion: 1, name: profile.label,
      fields: profile.fieldKinds.map(legacyProfileField)};
    put({id, kind: "readingTemplate", title: profile.label,
      sourceUri: `source://agents/skills/edit-structured-resources/assets/reading/${id}.json`,
      revision, content, reviewBaseline: content, pendingFieldIds: [], reviewedFieldIds: []});
    versions[key] = {resourceId: id, revision, source: structuredClone(profile)};
  }
}

export function resolveMockReadingProfile(
  versions: MockProfileVersions,
  reference: ReadingProfileRef,
  getVersion: (id: string, revision: number) => StructuredResource,
): StructuredResource {
  const version = versions[profileKey(reference)];
  if (!version) throw new Error(`旧阅读框架 ${reference.profileId}@${reference.profileVersion} 没有准确版本映射`);
  return getVersion(version.resourceId, version.revision);
}

export function readMockProfileSelections(storage: Storage): readonly PaperReadingProfileSelection[] {
  const raw = storage.getItem("research-assistant.paper-reading-profile-selections.v1");
  if (raw === null) return [];
  try {
    const values = decodeList(JSON.parse(raw) as unknown, decodePaperReadingProfileSelection);
    if (values) return values;
  } catch { /* Report the intact legacy record rather than replacing it. */ }
  throw new Error("本地阅读框架选择无法迁移；原始记录未被修改。");
}
