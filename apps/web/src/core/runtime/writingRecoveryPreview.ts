export const writingRecoveryScenarios = [
  { id: "discovery", label: "文件修改读取失败" },
  { id: "stale", label: "旧修改版本冲突" },
  { id: "application", label: "文件应用结果未确认" },
  { id: "acknowledgement", label: "交付记录确认失败" },
] as const;
export type WritingRecoveryScenario = typeof writingRecoveryScenarios[number]["id"];

export function readWritingRecoveryScenario(search: string): WritingRecoveryScenario | undefined {
  const params = new URLSearchParams(search);
  if (params.get("preview") !== "app" && params.get("demo") !== "app") return undefined;
  return writingRecoveryScenarios.find(scenario => scenario.id === params.get("writingRecovery"))?.id;
}

export function writingRecoveryPreviewUrl(scenario: WritingRecoveryScenario): string {
  return `/?preview=app&writingRecovery=${scenario}`;
}
