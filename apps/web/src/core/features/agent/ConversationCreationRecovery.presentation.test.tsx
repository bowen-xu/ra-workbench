import { renderToStaticMarkup } from "react-dom/server";
import { expect, it, vi } from "vitest";
import { ConversationCreationRecovery } from "./ConversationCreationRecovery";
import type { ConversationCreationSnapshot } from "./conversationCreationController";

const base: ConversationCreationSnapshot = {phase: "saving", intent: null, proof: null, status: null,
  current: null, cause: null, localIssue: null, adopted: false};
function render(state: ConversationCreationSnapshot) {
  return renderToStaticMarkup(<ConversationCreationRecovery state={state} check={vi.fn()} retryOriginal={vi.fn()}
    readCurrent={vi.fn()} finishKnown={vi.fn()} clearAdopted={vi.fn()} captureAdoption={() => vi.fn()} />);
}
it("hides ordinary creation and reading, but shows a real read failure", () => {
  expect(render(base)).toBe("");
  expect(render({...base, phase: "committing"})).toBe("");
  expect(render({...base, phase: "reading"})).toBe("");
  expect(render({...base, phase: "readError", cause: "unavailable"})).toContain("读取失败");
});
