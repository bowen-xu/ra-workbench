import { expect, it, vi } from "vitest";
import { createTextDocumentAdapter, type DocumentSnapshot } from "@research-assistant/core/hbs";
import { directTextDiff } from "../../../core/components/hbs/codeMirrorHbsPolicy";
import { HbsDeliveryPort } from "../../../core/components/hbs/deliveryPort";
import { resetExample } from "./resetExample";

const adapter = createTextDocumentAdapter(directTextDiff);
function deferred() { let resolve!: () => void; const promise = new Promise<void>(done => { resolve = done; }); return {promise, resolve}; }
function fixture() {
  let saved: DocumentSnapshot<string> = {version: 1, saved: "old edit", baseline: "old baseline"};
  let preservation: unknown = {content: "old preserved edit"};
  const actual = {read: vi.fn(async () => saved), commit: vi.fn(async (request: {expectedVersion: number; saved: string; baseline: string}) => {
    expect(request.expectedVersion).toBe(saved.version); saved = {version: saved.version + 1, saved: request.saved, baseline: request.baseline}; return saved;
  })};
  const session = {dispose: vi.fn(), flushPreservation: vi.fn(async () => {})};
  return {actual, session, adapter, fixture: "initial", delivery: new HbsDeliveryPort(actual), external: [],
    clearPreservation: vi.fn(async () => {preservation = null;}), readPreservation: vi.fn(async () => preservation)};
}

it("settles an already dispatched save, rejects queued/new writes, then resets against the latest actual version", async () => {
  const f = fixture(), dispatched = deferred(), checkpoint = deferred();
  const commit = f.actual.commit;
  f.actual.commit = vi.fn(async request => {await dispatched.promise; return commit(request);});
  f.delivery = new HbsDeliveryPort(f.actual);
  f.session.flushPreservation.mockImplementation(() => checkpoint.promise);
  const oldSave = f.delivery.commit({id: "old", expectedVersion: 1, saved: "late save", baseline: "late save"});
  f.delivery.setAutomatic(false);
  const queued = f.delivery.commit({id: "queued", expectedVersion: 2, saved: "queued", baseline: "queued"});
  const rejected = expect(queued).rejects.toThrow("MODEL_SESSION_CLOSED");
  const reset = resetExample(f);
  expect(f.session.dispose).toHaveBeenCalledOnce(); expect(f.actual.read).not.toHaveBeenCalled();
  await expect(f.delivery.commit({id: "new", expectedVersion: 2, saved: "new", baseline: "new"})).rejects.toThrow("MODEL_SESSION_CLOSED");
  dispatched.resolve(); await oldSave; await rejected;
  expect(f.clearPreservation).not.toHaveBeenCalled();
  checkpoint.resolve(); await reset;
  expect(await f.actual.read()).toEqual({version: 3, saved: "initial", baseline: "initial"});
  expect(f.actual.commit.mock.calls.map(([request]) => request.saved)).toEqual(["late save", "initial"]);
  expect(await f.readPreservation()).toBeNull();
});

it("waits for an external operation before replacing S/B", async () => {
  const f = fixture(), external = deferred();
  const reset = resetExample({...f, external: [external.promise]});
  expect(f.actual.read).not.toHaveBeenCalled(); external.resolve(); await reset;
  expect((await f.actual.read()).saved).toBe("initial");
});

it("keeps reset failure explicit and a retry completes cleanup without repeating a confirmed reset write", async () => {
  const f = fixture(); f.clearPreservation.mockRejectedValueOnce(new Error("disk busy"));
  await expect(resetExample(f)).rejects.toThrow("disk busy");
  expect(f.actual.commit).toHaveBeenCalledTimes(1);
  await resetExample(f);
  expect(f.actual.commit).toHaveBeenCalledTimes(1); expect(await f.readPreservation()).toBeNull();
});

it("does not claim success if persisted S/B or preservation fails verification", async () => {
  const f = fixture(); f.clearPreservation.mockImplementation(async () => {});
  await expect(resetExample(f)).rejects.toThrow("MODEL_RESET_VERIFY_FAILED");
});
