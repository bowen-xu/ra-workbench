/** @vitest-environment happy-dom */
import { act } from "react";
import { expect, it, vi } from "vitest";
import { Text } from "@codemirror/state";
import { undo, redo } from "@codemirror/commands";
import { getChunks, getOriginalDoc } from "@codemirror/merge";
import { initialChanges } from "../../../core/features/agent/WritingDiffFixtures";
import * as f from "./WritingDiffDemoAgentStorage.integrationHarness";
import { scope, mount, view, settled } from "./WritingDiffDemoAgentStorage.integrationHarness";

async function click(label: string) {
  const button = f.host.querySelector<HTMLButtonElement>(`[aria-label="${label}"]`);
  expect(button).not.toBeNull();
  await act(async () => button!.click());
}

it.each(["保留第 1 处变更", "撤销第 1 处变更", "Keep all", "Undo all"])(
  "preserves %s as one reversible file transaction through preview persistence and restart", async label => {
    const current = await mount();
    await settled(() => expect(f.documents.agentDelivery.pending).toBe(false));
    const background = f.documents.files.all().slice(1).map(file => file.read().content);
    expect(getChunks(current.state)?.chunks).toHaveLength(2);
    await click(label);
    const content = current.state.doc.toString(), baseline = getOriginalDoc(current.state).toString();
    const count = label.endsWith("all") ? 0 : 1;
    expect(getChunks(current.state)?.chunks).toHaveLength(count);
    await act(async () => { expect(undo(current)).toBe(true); });
    expect(current.state.doc.toString()).toBe(initialChanges[0]!.after);
    expect(getOriginalDoc(current.state).toString()).toBe(initialChanges[0]!.before);
    expect(getChunks(current.state)?.chunks).toHaveLength(2);
    await act(async () => { expect(redo(current)).toBe(true); });
    expect(current.state.doc.toString()).toBe(content);
    expect(getOriginalDoc(current.state).toString()).toBe(baseline);
    expect(f.documents.files.all().slice(1).map(file => file.read().content)).toEqual(background);
    await act(async () => { expect(await f.documents.restart()).toBe(true); });
    await settled(() => expect(f.documents.ready && !f.documents.agentDelivery.pending).toBe(true));
    expect(view()).not.toBe(current);
    expect(view().state.doc.toString()).toBe(content);
    expect(getOriginalDoc(view().state).toString()).toBe(baseline);
    expect(getChunks(view().state)?.chunks).toHaveLength(count);
    expect((await f.storage.files.readFile(scope.manuscriptId, "main.tex")).content).toBe(content);
    expect(f.documents.files.all().slice(1).map(file => file.read().content)).toEqual(background);
  });

it("undoes and redoes both Agent changes together without touching other preview files", async () => {
  const current = await mount();
  await settled(() => expect(f.documents.agentDelivery.pending).toBe(false));
  const background = f.documents.files.all().slice(1).map(file => file.read().content);
  await act(async () => { expect(undo(current)).toBe(true); });
  expect(current.state.doc.toString()).toBe(initialChanges[0]!.before);
  expect(getChunks(current.state)?.chunks).toHaveLength(0);
  await act(async () => { expect(redo(current)).toBe(true); });
  expect(current.state.doc.toString()).toBe(initialChanges[0]!.after);
  expect(getOriginalDoc(current.state).toString()).toBe(initialChanges[0]!.before);
  expect(getChunks(current.state)?.chunks).toHaveLength(2);
  expect(f.documents.files.all().slice(1).map(file => file.read().content)).toEqual(background);
});

it("tracks the preview cursor and stops diff navigation at both boundaries", async () => {
  const current = await mount();
  const chunks = getChunks(current.state)!.chunks;
  await act(async () => current.dispatch({ selection: { anchor: chunks[1]!.fromB } }));
  expect(f.host.querySelector('[aria-label="当前变更位置"]')?.textContent).toBe("2/2");
  const second = current.state.selection.main.head;
  await click("下一处变更");
  expect(current.state.selection.main.head).toBe(second);
  await click("上一处变更");
  expect(current.state.selection.main.head).toBe(chunks[0]!.fromB);
  expect(f.host.querySelector('[aria-label="当前变更位置"]')?.textContent).toBe("1/2");
  await click("上一处变更");
  expect(current.state.selection.main.head).toBe(chunks[0]!.fromB);
  expect(view()).toBe(current);
});

it.each([false, true])("restores Keep after preview close with saved-before-close=%s", async waitForSave => {
  const current = await mount();
  await settled(() => expect(f.documents.agentDelivery.pending).toBe(false));
  await click("Keep all");
  if (waitForSave) await vi.waitFor(async () => {
    await act(async () => { await Promise.resolve(); });
    const saved = (await f.storage.checkpoints.list(scope.manuscriptId)).find(record => record.checkpoint.relativePath === "main.tex")!.checkpoint;
    const live = f.documents.files.get({ manuscriptId: scope.manuscriptId, relativePath: "main.tex" })!.read();
    expect(saved.version).toBe(live.version);
    expect(saved.savedVersion).toBe(live.version);
    expect(saved.reviewBaseline).toBe(initialChanges[0]!.after);
    expect(saved.saveIntent).toBeNull();
    expect(f.documents.saveState("main.tex")).toBe("saved");
  });
  await act(async () => { expect(await f.documents.restart()).toBe(true); });
  await settled(() => expect(f.documents.ready && !f.documents.agentDelivery.pending).toBe(true));
  expect(view()).not.toBe(current);
  expect(view().state.doc.toString()).toBe(initialChanges[0]!.after);
  expect(getOriginalDoc(view().state).toString()).toBe(initialChanges[0]!.after);
  expect(getChunks(view().state)?.chunks).toHaveLength(0);
  const checkpoint = (await f.storage.checkpoints.list(scope.manuscriptId)).find(record => record.checkpoint.relativePath === "main.tex")!;
  expect(checkpoint.checkpoint.savedVersion).toBe(checkpoint.checkpoint.version);
  expect((await f.storage.files.readFile(scope.manuscriptId, "main.tex")).content).toBe(initialChanges[0]!.after);
});

it("persists CRLF and Unicode outside pending chunks without creating another diff", async () => {
  await f.agent.initialized;
  const before = "heading\r\nold words\r\nfooter", after = "heading\r\nnew words\r\nfooter";
  f.configure([{ ...initialChanges[0]!, before, after }, ...initialChanges.slice(1)]);
  const current = await mount(), position = after.indexOf("footer");
  await act(async () => current.dispatch({ changes: { from: position, to: position + 6, insert: Text.of(["替换😀\r", "next"]) }, userEvent: "input.paste" }));
  const content = "heading\r\nnew words\r\n替换😀\r\nnext", baseline = "heading\r\nold words\r\n替换😀\r\nnext";
  expect(current.state.doc.toString()).toBe(content);
  expect(getOriginalDoc(current.state).toString()).toBe(baseline);
  expect(getChunks(current.state)?.chunks).toHaveLength(1);
  await act(async () => { expect(undo(current)).toBe(true); });
  expect(current.state.doc.toString()).toBe(after);
  expect(getOriginalDoc(current.state).toString()).toBe(before);
  await act(async () => { expect(redo(current)).toBe(true); });
  await act(async () => { expect(await f.documents.restart()).toBe(true); });
  await settled(() => expect(f.documents.ready && !f.documents.agentDelivery.pending).toBe(true));
  expect(view().state.doc.toString()).toBe(content);
  expect(getOriginalDoc(view().state).toString()).toBe(baseline);
  expect(getChunks(view().state)?.chunks).toHaveLength(1);
});

const middle = "\nunchanged one\nunchanged two\nunchanged three\n";
it.each([
  { name: "start boundary", before: "heading\nold words\nfooter", after: "heading\nnew words\nfooter", from: 8, to: 8,
    insert: "边界😀\r\n继续", baseline: "heading\nold words\nfooter" },
  { name: "end boundary", before: "heading\nold words\nfooter", after: "heading\nnew words\nfooter", from: 17, to: 17,
    insert: "边界😀\r\n继续", baseline: "heading\nold words\nfooter" },
  { name: "cross-chunk replacement", before: `first old${middle}last old`, after: `first new${middle}last new`, from: 6, to: `first new${middle}last new`.length,
    insert: "替换😀\r\n多行", baseline: "first old\nlast old" },
  { name: "pure deletion crossing", before: "prefix\ndeleted\nstable\ntail", after: "prefix\nstable\ntail", from: 0, to: 14,
    insert: "", baseline: "deleted\ntail" },
])("restores mapped review bytes after preview $name, history and restart", async change => {
  await f.agent.initialized;
  f.configure([{ ...initialChanges[0]!, before: change.before, after: change.after }, ...initialChanges.slice(1)]);
  const current = await mount();
  if (change.name === "start boundary") expect(change.from).toBe(getChunks(current.state)!.chunks[0]!.fromB);
  if (change.name === "end boundary") expect(change.from).toBe(getChunks(current.state)!.chunks[0]!.endB);
  const edited = change.after.slice(0, change.from) + change.insert + change.after.slice(change.to);
  await act(async () => current.dispatch({ changes: { from: change.from, to: change.to, insert: Text.of(change.insert.split("\n")) }, userEvent: "input.paste" }));
  expect(current.state.doc.toString()).toBe(edited);
  expect(getOriginalDoc(current.state).toString()).toBe(change.baseline);
  expect(getChunks(current.state)?.chunks).toHaveLength(1);
  await act(async () => { expect(undo(current)).toBe(true); });
  expect(current.state.doc.toString()).toBe(change.after);
  expect(getOriginalDoc(current.state).toString()).toBe(change.before);
  await act(async () => { expect(redo(current)).toBe(true); });
  await act(async () => { expect(await f.documents.restart()).toBe(true); });
  await settled(() => expect(f.documents.ready && !f.documents.agentDelivery.pending).toBe(true));
  expect(view().state.doc.toString()).toBe(edited);
  expect(getOriginalDoc(view().state).toString()).toBe(change.baseline);
  expect((await f.storage.files.readFile(scope.manuscriptId, "main.tex")).content).toBe(edited);
});
