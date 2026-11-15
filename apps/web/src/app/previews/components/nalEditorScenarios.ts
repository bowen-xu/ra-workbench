import { emptyNalState, type NalConcept, type NalPattern, type NalQuery, type NalQueryBlock, type NalVocabulary } from "@research-assistant/core/nal";
import { newStatementDraft } from "../../../../../../plugins/literature_library/src/features/nal/nalQueryBuilderModel";

const concepts: readonly Pick<NalConcept, "id" | "label" | "conceptKind">[] = [
  { id: "ai", label: "AI领域" }, { id: "deep-learning", label: "深度学习领域" },
  { id: "active-inference", label: "主动推理" }, { id: "domain", conceptKind: "relation", label: "所属领域" },
  { id: "method", conceptKind: "relation", label: "基于方法" }, { id: "development", conceptKind: "relation", label: "研究发展" },
];
export const nalEditorState = { ...emptyNalState(), concepts: concepts.map(concept => ({ ...concept, explanation: "", revision: 0 })) };
export const nalEditorVocabulary: readonly NalVocabulary[] = [
  ...concepts.map(concept => ({ atom: { kind: "concept" as const, id: concept.id }, label: concept.label })),
  { atom: { kind: "paper", id: "paper-vision" }, label: "Vision by Logic" },
  { atom: { kind: "paper", id: "paper-inference" }, label: "Active Inference 示例文献" },
];
const variable = (name: string): NalPattern => ({ kind: "variable", name });
const concept = (id: string): NalPattern => ({ kind: "concept", id });
function relation(target: NalPattern, predicate = "domain"): NalQuery {
  return { copula: "inheritance", subject: { kind: "product", terms: [variable("文章"), target] }, predicate: concept(predicate) };
}

export interface NalEditorScenario {
  readonly id: string;
  readonly label: string;
  readonly description: string;
  readonly create: () => readonly NalQueryBlock[];
}
export const nalEditorScenarios: readonly NalEditorScenario[] = [
  { id: "relation", label: "关系查询", description: "单击词项或连接符打开菜单，点选即确认；在乘积内部拖动词项调整顺序。", create: () => [relation(concept("ai"))] },
  { id: "blank", label: "从空白开始", description: "单击选择连接符，可建立继承、蕴含语句或 conjunction／disjunction 分组。", create: () => [newStatementDraft()] },
  { id: "groups", label: "嵌套分组", description: "行前 + 在当前块之后添加同级块；可包裹分组，并切换 conjunction / disjunction。", create: () => [
    { kind: "group", connector: "disjunction", children: [relation(concept("deep-learning")),
      { kind: "group", connector: "conjunction", children: [relation(concept("ai")), relation(variable("方法"), "method")] }] },
  ] },
  { id: "statement", label: "嵌套语句", description: "左右槽都能在 atomic、compound、statement 之间切换；单击嵌套槽位继续选择。", create: () => [
    { copula: "implication", subject: { kind: "statement", ...relation(variable("方法"), "method") },
      predicate: { kind: "statement", ...relation(concept("ai")) } },
  ] },
];
