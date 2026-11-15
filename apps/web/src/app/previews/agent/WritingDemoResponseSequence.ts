import type { AgentWorkspaceBuffer } from "@research-assistant/core/agent";
import { formatSourceLink } from "@research-assistant/core/source";
import { WritingDemoFeedback } from "./WritingDemoFeedback";
import type { DemoChange } from "../../../core/features/agent/WritingDiffFixtures";
import type { WritingDiffDemoHistory } from "./WritingDiffDemoHistory";
import type { WritingSubmissionAdmission } from "../../../core/features/agent/writingSubmissionController";
import type { WritingDemoTextPacing } from "./WritingDemoTextStream";
import { writingDemoPatch } from "./WritingDemoPatchContract";
import { writingDemoResponseMessages } from "./WritingDemoMessages";

export type WritingDemoResponseTexts = readonly [string, string, string, string];
export type WritingDemoPatchFailure = "persistent" | "once";

/** A model/tool fixture over an execution copy. Formal files remain owned by
 * the production delivery path, even when this turn edits a file repeatedly. */
export class WritingDemoResponseSequence {
  private index = 0;
  private feedback: WritingDemoFeedback | undefined;
  private readonly contents: Map<string, string>;
  constructor(private readonly buffers: readonly AgentWorkspaceBuffer[], private readonly progressive = false,
    private readonly responseTexts?: WritingDemoResponseTexts, private readonly pacing?: () => WritingDemoTextPacing,
    private readonly patchFailure?: WritingDemoPatchFailure) {
    this.contents = new Map(buffers.map(buffer => [buffer.path, buffer.text]));
  }
  private get stepCount(): number { return this.patchFailure === "persistent" ? 3 : 4; }
  get remainingSteps(): number { return this.feedback?.remainingSteps ?? this.stepCount - this.index; }
  get remainingWaitSteps(): number {
    return (this.stepCount - this.index - (this.feedback ? 1 : 0)) * WritingDemoFeedback.waitSteps
      + (this.feedback?.remainingWaitSteps ?? 0);
  }
  get isStreaming(): boolean { return this.feedback?.isStreaming ?? false; }
  get atModelBoundary(): boolean { return this.feedback?.atModelBoundary ?? this.index > 0; }
  restartIncompleteResponse(): void { this.feedback?.restartIncompleteResponse(); }
  nextDelayMs(waitMs: number): number { return this.feedback?.nextDelayMs(waitMs) ?? waitMs; }
  next(history: WritingDiffDemoHistory, admission: WritingSubmissionAdmission): boolean {
    if (this.feedback) {
      const output = this.feedback.next(history, admission);
      if (!this.feedback.remainingSteps) { this.feedback = undefined; this.index++; }
      return output;
    }
    const respond = (step: ConstructorParameters<typeof WritingDemoFeedback>[0]) => {
      const phased = { ...step, phase: this.index === this.stepCount - 1 ? "finalAnswer" as const : "commentary" as const };
      if (this.progressive) { this.feedback = new WritingDemoFeedback(phased, this.pacing); this.feedback.next(history, admission); }
      else history.responseStep(admission, phased);
    };
    const file = this.buffers[0];
    if (!file || this.remainingSteps === 0) return false;
    const before = this.contents.get(file.path)!;
    if (this.index === 0) {
      respond({ name: "research_read_file", arguments: { path: `source://writings/${admission.scope.manuscriptId}/${file.path}` },
        output: { text: before, startChar: 0, nextChar: null }, text: this.patchFailure
          ? "已读取当前稿件，接下来尝试修改。" : this.responseTexts?.[0] ?? writingDemoResponseMessages[0] });
    } else if (this.patchFailure && this.index === 1) {
      const patch = writingDemoPatch(admission.scope.manuscriptId, [{ path: file.path, kind: "modified", before, after: before + "\n% Mock Agent: update\n" }]);
      respond({ ...patch, isError: true, output: { status: "failed", code: "file_write_failed",
        targets: [{ path: formatSourceLink({ path: ["writings", admission.scope.manuscriptId, ...file.path.split("/")] }),
          name: file.path, status: "failed", message: "模拟文件写入失败；本次调用未修改文件。" }] },
        text: this.patchFailure === "once"
          ? "修改工具返回写入失败，且确认没有写入。我会重试一次。"
          : "修改工具返回写入失败。我会重新读取文件，确认本轮是否发生了修改。" });
    } else if (this.patchFailure === "persistent") {
      respond({ name: "research_read_file", arguments: { path: `source://writings/${admission.scope.manuscriptId}/${file.path}` },
        output: { text: before, startChar: 0, nextChar: null },
        text: "本轮修改失败：文件写入失败。重新读取后确认，文件仍是修改前的内容，本轮没有产生文件改动。" });
    } else if (this.index === 1 || this.index === 2) {
      const keywords = "\\keywords{research, agent, reproducibility}";
      const after = this.patchFailure === "once" ? before + "\n% Mock Agent: update\n" : this.index === 1
        ? (/\\keywords\{[^}]*\}/u.test(before) ? before.replace(/\\keywords\{[^}]*\}/u, keywords) : before + `\n${keywords}\n`)
          + "\n% Mock Agent: keywords updated\n"
        : before + "\\begin{abstract}\nThis mock manuscript studies reproducible research agents.\n\\end{abstract}\n";
      this.contents.set(file.path, after);
      respond({ ...writingDemoPatch(admission.scope.manuscriptId, [{ path: file.path, kind: "modified", before, after }]), text: this.patchFailure === "once"
        ? "重试成功，修改工具已确认写入。接下来检查结果。" : this.responseTexts?.[this.index] ?? writingDemoResponseMessages[this.index === 1 ? 1 : 2] });
    } else {
      respond({ name: "research_exec_command", arguments: { command: "mock-validate-manuscript" },
        output: { state: "finished", exitCode: 0, output: this.patchFailure ? "retry result validated" : "keywords and abstract validated" },
        text: this.patchFailure ? "检查通过，本轮修改已完成。" : this.responseTexts?.[3] ?? writingDemoResponseMessages[3] });
    }
    if (!this.progressive) this.index++;
    return !this.progressive;
  }
  changes(): readonly DemoChange[] {
    return this.buffers.flatMap(buffer => {
      const after = this.contents.get(buffer.path)!;
      return after === buffer.text ? [] : [{ path: buffer.path, kind: "modified" as const,
        before: buffer.text, after, additions: this.patchFailure ? 1 : 6, deletions: this.patchFailure ? 0 : /\\keywords\{[^}]*\}/u.test(buffer.text) ? 1 : 0 }];
    });
  }
}
