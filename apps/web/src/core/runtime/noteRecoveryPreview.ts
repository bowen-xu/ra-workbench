export const noteRecoveryGroups = [{label: "冲突与消解", cases: [
  ["conflict", "正文并发冲突", "同一候选被本地和另一写入者分别修改，重开后由 HBS 重建冲突。"],
  ["resolution-failure", "冲突消解事务失败", "正文内 Keep／Undo 后，真实 HBS 提交在写入前失败。"],
  ["resolution-unknown", "冲突消解回执丢失", "正文内 Keep／Undo 已保存，但回执持续丢失；核对原 HBS 请求。"],
  ["stale-resolution", "消解期间版本变化", "内嵌决定提交前另一写入者先更新 S，真实 HBS 重建冲突。"],
]}, {label: "保存与恢复", cases: [
  ["save-failure", "保存事务失败", "正文提交在写入前持续失败；解除模拟故障后，通过原请求重试。"],
  ["lost-receipt", "保存回执丢失一次", "事务已成功，但首次回执丢失；重开保留原请求，重试可核对结果。"],
  ["stuck-request", "原请求持续无回执", "每次核对都丢失回执；不会通过选择正文绕过原请求。"],
  ["checkpoint-read", "恢复记录读取失败", "S′ 读取接口持续失败；真实会话保护已有记录，不用权威正文覆盖它。"],
  ["checkpoint-invalid", "恢复记录结构损坏", "读取接口返回损坏的 mock S′，由真实解码器拒绝。"],
  ["checkpoint-write", "恢复保全写入失败", "恢复已有输入后，S′ 写入接口失败，阻止后续正式提交。"],
]}] as const;
export const noteRecoveryCases = noteRecoveryGroups.flatMap(group => [...group.cases]);
export type NoteRecoveryScenario = typeof noteRecoveryCases[number][0];
export function readNoteRecoveryScenario(search: string): NoteRecoveryScenario | undefined {
  const value = new URLSearchParams(search).get("note-recovery");
  return noteRecoveryCases.find(item => item[0] === value)?.[0];
}
export interface NoteRecoveryPreview {
  readonly scenario: NoteRecoveryScenario;
  readonly pageId: string;
  releaseFault(): Promise<void>;
}
