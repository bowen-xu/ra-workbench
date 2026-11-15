import { describe, expect, it, vi } from "vitest";

import { noteBlockId, notePageId, type NoteDocumentV1, type NotePage } from "@research-assistant/core/notes";
import { NotesGatewayError, type NoteDocumentSnapshot, type NotesGateway } from "@research-assistant/data/notes";

import { NoteSaveController, type NoteSaveStatus } from "./noteSaveController";

function deferred<T>(): { readonly promise: Promise<T>; readonly resolve: (value: T) => void } {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

const pageId = notePageId("page-1");
function document(text: string): NoteDocumentV1 {
  return { schemaVersion: 1, pageId, blocks: [{ id: noteBlockId("block-1"), type: "paragraph", props: {}, content: { kind: "inline", items: [{ type: "text", text, styles: [] }] }, children: [] }] };
}

function snapshot(value: NoteDocumentV1, revision: number): NoteDocumentSnapshot {
  const page: NotePage = { id: pageId, title: "Note", position: 0, status: "active", isFavorite: false, revision, schemaVersion: 1, projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now" };
  return { page, document: value };
}

describe("NoteSaveController", () => {
  it("waits for three uninterrupted idle seconds and flushes immediately at a switch boundary", async () => {
    vi.useFakeTimers();
    const f = externalFixture();
    try {
      for (let index = 0; index < 5; index++) {
        f.controller.enqueue(document(`edit ${index}`));
        await vi.advanceTimersByTimeAsync(1000);
      }
      expect(f.saveDocument).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(1999);
      expect(f.saveDocument).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(1);
      expect(f.saveDocument).toHaveBeenCalledOnce();
      expect(f.saveDocument.mock.calls[0]?.[0].document).toEqual(document("edit 4"));
      f.controller.enqueue(document("switch now"));
      await f.controller.flush();
      expect(f.saveDocument).toHaveBeenCalledTimes(2);
      await vi.advanceTimersByTimeAsync(3000);
      expect(f.saveDocument).toHaveBeenCalledTimes(2);
    } finally { f.controller.dispose(); vi.useRealTimers(); }
  });

  it("keeps the new idle deadline when typing resumes during an automatic write", async () => {
    vi.useFakeTimers();
    const f = externalFixture(), first = deferred<NoteDocumentSnapshot>();
    f.saveDocument.mockImplementationOnce(() => first.promise);
    try {
      f.controller.enqueue(document("first"));
      await vi.advanceTimersByTimeAsync(3000);
      f.controller.enqueue(document("second"));
      await vi.advanceTimersByTimeAsync(1000);
      first.resolve(snapshot(document("first"), 1));
      await vi.advanceTimersByTimeAsync(0);
      expect(f.saveDocument).toHaveBeenCalledOnce();
      await vi.advanceTimersByTimeAsync(1999);
      expect(f.saveDocument).toHaveBeenCalledOnce();
      await vi.advanceTimersByTimeAsync(1);
      expect(f.saveDocument).toHaveBeenCalledTimes(2);
      expect(f.saveDocument.mock.calls[1]?.[0]).toMatchObject({ document: document("second"), expectedRevision: 1 });
    } finally { f.controller.dispose(); vi.useRealTimers(); }
  });

  it("serializes saves and coalesces edits made while a save is in flight", async () => {
    const first = deferred<NoteDocumentSnapshot>();
    const calls: { document: NoteDocumentV1; expectedRevision: number }[] = [];
    const saveDocument = vi.fn(async (request: { document: NoteDocumentV1; expectedRevision: number }) => {
      calls.push(request);
      if (calls.length === 1) return first.promise;
      return snapshot(request.document, 2);
    });
    const statuses: NoteSaveStatus[] = [];
    const visibleSnapshots: NoteDocumentSnapshot[] = [];
    const controller = new NoteSaveController({pageId,
      gateway: { saveDocument } as unknown as NotesGateway,
      initialRevision: 0,
      onSnapshot: (value) => visibleSnapshots.push(value),
      onStatus: (status) => statuses.push(status),
    });
    controller.enqueue(document("first"));
    const flushing = controller.flush();
    controller.enqueue(document("latest"));
    first.resolve(snapshot(document("first"), 1));
    await flushing;
    expect(visibleSnapshots[0]?.document).toEqual(document("latest"));
    expect(calls).toHaveLength(2);
    expect(calls[1]?.expectedRevision).toBe(1);
    expect(calls[1]?.document.blocks[0]?.content).toEqual(document("latest").blocks[0]?.content);
    expect(statuses.at(-1)).toBe("saved");
  });

  it("stops on revision conflict until the caller resolves it", async () => {
    const saveDocument = vi.fn().mockRejectedValueOnce(new NotesGatewayError("revision_conflict", "conflict", 3))
      .mockResolvedValueOnce(snapshot(document("mine"), 4));
    const statuses: NoteSaveStatus[] = [];
    const controller = new NoteSaveController({pageId,
      gateway: { saveDocument } as unknown as NotesGateway,
      initialRevision: 0,
      onSnapshot: () => undefined,
      onStatus: (status) => statuses.push(status),
    });
    controller.enqueue(document("mine"));
    await controller.flush();
    expect(saveDocument).toHaveBeenCalledTimes(1);
    expect(statuses.at(-1)).toBe("conflict");
    expect(controller.hasUnsavedChanges()).toBe(true);
    await controller.flush();
    expect(saveDocument).toHaveBeenCalledTimes(1);
    controller.updateRevision(3);
    await controller.retry();
    expect(saveDocument).toHaveBeenCalledTimes(2);
    expect(statuses.at(-1)).toBe("saved");
    expect(controller.hasUnsavedChanges()).toBe(false);
  });

  it("reports failed pending content so document replacement cannot discard it", async () => {
    const saveDocument = vi.fn().mockRejectedValue(new Error("disk full"));
    const controller = new NoteSaveController({pageId,
      gateway: { saveDocument } as unknown as NotesGateway,
      initialRevision: 0,
      onSnapshot: () => undefined,
      onStatus: () => undefined,
    });
    controller.enqueue(document("must survive"));
    await controller.flush();
    expect(controller.hasUnsavedChanges()).toBe(true);
    expect(saveDocument).toHaveBeenCalledOnce();
  });

  it("marks recovery durable only after the newest queued document is saved", async () => {
    const first = deferred<NoteDocumentSnapshot>();
    const saveDocument = vi.fn()
      .mockImplementationOnce(() => first.promise)
      .mockImplementationOnce((request: { document: NoteDocumentV1 }) => snapshot(request.document, 2));
    const durable = vi.fn();
    const controller = new NoteSaveController({pageId,
      gateway: { saveDocument } as unknown as NotesGateway,
      initialRevision: 0,
      onSnapshot: () => undefined,
      onDurable: durable,
      onStatus: () => undefined,
    });

    controller.enqueue(document("first"));
    const flushing = controller.flush();
    controller.enqueue(document("latest"));
    first.resolve(snapshot(document("first"), 1));
    await flushing;

    expect(durable).toHaveBeenCalledOnce();
    expect(durable.mock.calls[0]?.[0].document).toEqual(document("latest"));
  });

  it("restores a conflicting draft without overwriting the durable version", async () => {
    const saveDocument = vi.fn();
    const statuses: NoteSaveStatus[] = [];
    const controller = new NoteSaveController({pageId,
      gateway: { saveDocument } as unknown as NotesGateway,
      initialRevision: 3,
      onSnapshot: () => undefined,
      onStatus: (status) => statuses.push(status),
    });

    controller.restore(document("recovered"), "conflict");
    await controller.flush();

    expect(controller.hasUnsavedChanges()).toBe(true);
    expect(saveDocument).not.toHaveBeenCalled();
    expect(statuses.at(-1)).toBe("conflict");
  });
});

function externalFixture() {
  const onSnapshot = vi.fn(), onDurable = vi.fn(), onStatus = vi.fn();
  const saveDocument = vi.fn(async (request: {document: NoteDocumentV1; expectedRevision: number}) => snapshot(request.document, request.expectedRevision + 1));
  const controller = new NoteSaveController({pageId,gateway: {saveDocument} as unknown as NotesGateway,
    initialRevision: 0, onSnapshot, onDurable, onStatus});
  return {controller, saveDocument, onSnapshot, onDurable, onStatus};
}

it("holds autosave admission until native acceptance and synchronous editor installation complete", async () => {
  const f = externalFixture();
  const lease = f.controller.beginExternalChange(0, document("before"))!;
  expect(lease).not.toBeNull();expect(f.controller.hasUnsavedChanges()).toBe(true);
  expect(f.controller.beginExternalChange(0, document("before"))).toBeNull();
  await f.controller.flush();await f.controller.retry();expect(f.saveDocument).not.toHaveBeenCalled();
  expect(lease.markDispatched()).toBe(true);expect(lease.markDispatched()).toBe(false);
  const install = vi.fn(() => {expect(f.onSnapshot).not.toHaveBeenCalled();return undefined;});
  expect(lease.commit(snapshot(document("accepted"), 1), install)).toBe(true);
  expect(install).toHaveBeenCalledOnce();expect(f.onDurable).toHaveBeenCalledOnce();
  expect(f.controller.hasUnsavedChanges()).toBe(false);
  expect(lease.commit(snapshot(document("accepted"), 1), install)).toBe(false);
  f.controller.enqueue(document("later input"));await f.controller.flush();
  expect(f.saveDocument).toHaveBeenCalledWith({document: document("later input"), expectedRevision: 1, saveReason: "autosave"});
});

it("refuses acquisition over pending, in-flight, failed or disposed saves and different revisions", async () => {
  const f = externalFixture();
  expect(f.controller.beginExternalChange(0, {...document("before"), pageId: notePageId("another-page")})).toBeNull();
  expect(f.controller.beginExternalChange(1, document("before"))).toBeNull();
  f.controller.enqueue(document("unsaved"));expect(f.controller.beginExternalChange(0, document("before"))).toBeNull();
  const pending = deferred<NoteDocumentSnapshot>();f.saveDocument.mockImplementationOnce(() => pending.promise);
  const saving = f.controller.flush();expect(f.controller.beginExternalChange(0, document("before"))).toBeNull();
  pending.resolve(snapshot(document("unsaved"), 1));await saving;
  f.saveDocument.mockRejectedValueOnce(new Error("disk full"));f.controller.enqueue(document("failed"));await f.controller.flush();
  expect(f.controller.beginExternalChange(1, document("failed"))).toBeNull();f.controller.dispose();
  expect(f.controller.beginExternalChange(1, document("failed"))).toBeNull();
});

it("retains unexpected input as conflict rather than saving it over native changes", async () => {
  const f = externalFixture();const lease = f.controller.beginExternalChange(0, document("before"))!;
  lease.markDispatched();f.controller.enqueue(document("late input"));
  const install = vi.fn();expect(lease.commit(snapshot(document("accepted"), 1), install)).toBe(false);
  await f.controller.flush();await f.controller.retry();f.controller.updateRevision(1);await f.controller.retry();
  expect(f.saveDocument).not.toHaveBeenCalled();expect(install).not.toHaveBeenCalled();expect(f.onDurable).not.toHaveBeenCalled();
  expect(f.controller.hasUnsavedChanges()).toBe(true);expect(f.onStatus.mock.calls.at(-1)?.[0]).toBe("conflict");f.controller.dispose();
});

it("keeps uncertain dispatched work fenced and permits adoption only after reconciliation", async () => {
  const f = externalFixture();const lease = f.controller.beginExternalChange(0, document("before"))!;
  expect(lease.markDispatched()).toBe(true);lease.requireRecovery();
  expect(lease.cancelBeforeDispatch()).toBe(false);await f.controller.retry();
  expect(f.saveDocument).not.toHaveBeenCalled();expect(f.controller.beginExternalChange(0, document("before"))).toBeNull();
  expect(lease.commit(snapshot(document("accepted"), 1), () => undefined)).toBe(true);
  expect(f.controller.hasUnsavedChanges()).toBe(false);
});

it("cancels only before dispatch and never lets an old lease modify a disposed host", () => {
  const f = externalFixture();const cancelled = f.controller.beginExternalChange(0, document("before"))!;
  expect(cancelled.cancelBeforeDispatch()).toBe(true);expect(cancelled.markDispatched()).toBe(false);
  const lease = f.controller.beginExternalChange(0, document("before"))!;lease.markDispatched();f.controller.dispose();
  const install = vi.fn();expect(lease.commit(snapshot(document("accepted"), 1), install)).toBe(false);
  lease.requireRecovery();expect(install).not.toHaveBeenCalled();expect(f.onSnapshot).not.toHaveBeenCalled();
});

it("refuses mismatched snapshots and keeps installation failures fenced", () => {
  const f = externalFixture();const lease = f.controller.beginExternalChange(0, document("before"))!;const install = vi.fn();
  expect(lease.commit(snapshot(document("accepted"), 1), install)).toBe(false);lease.markDispatched();
  const other = snapshot(document("accepted"), 1);
  expect(lease.commit({...other, page: {...other.page, id: notePageId("other")}}, install)).toBe(false);
  expect(lease.commit({...other, document: {...other.document, pageId: notePageId("other")}}, install)).toBe(false);
  expect(lease.commit(snapshot(document("accepted"), 2), install)).toBe(false);expect(install).not.toHaveBeenCalled();
  expect(() => lease.commit(other, () => {throw new Error("installation failed");})).toThrow("installation failed");
  expect(f.onSnapshot).not.toHaveBeenCalled();expect(f.controller.hasUnsavedChanges()).toBe(true);
  expect(lease.commit(other, () => {f.controller.enqueue(document("unexpected onChange"));})).toBe(false);
  expect(f.onDurable).not.toHaveBeenCalled();f.controller.dispose();
});

it("does not flush retained conflict input through a disposed save controller", async () => {
  const f = externalFixture();const lease = f.controller.beginExternalChange(0, document("before"))!;
  lease.markDispatched();f.controller.restore(document("retained input"), "dirty");f.controller.dispose();
  await f.controller.retry();await f.controller.flush();f.controller.updateRevision(1);
  expect(f.saveDocument).not.toHaveBeenCalled();expect(f.onSnapshot).not.toHaveBeenCalled();
});

it("never acknowledges a mistakenly asynchronous editor installation", () => {
  const f = externalFixture();const lease = f.controller.beginExternalChange(0, document("before"))!;lease.markDispatched();
  const invalidInstaller = (() => Promise.resolve()) as unknown as () => undefined;
  expect(lease.commit(snapshot(document("accepted"), 1), invalidInstaller)).toBe(false);
  expect(f.onDurable).not.toHaveBeenCalled();expect(f.onSnapshot).not.toHaveBeenCalled();
  expect(f.controller.hasUnsavedChanges()).toBe(true);f.controller.dispose();
});
it("releases a definitively rejected navigation write without admitting input that invalidated its lease", async () => {
  const f = externalFixture();
  try {
    const rejected = f.controller.beginExternalChange(0, document("before"))!;
    rejected.markDispatched(); expect(rejected.rejectDispatched?.()).toBe(true);
    f.controller.enqueue(document("normal edit")); await f.controller.flush();
    expect(f.saveDocument).toHaveBeenCalledOnce();
    const stale = f.controller.beginExternalChange(1, document("normal edit"))!;
    stale.markDispatched(); f.controller.enqueue(document("later input"));
    expect(stale.rejectDispatched?.()).toBe(false);
    expect(f.controller.hasUnsavedChanges()).toBe(true);
  } finally { f.controller.dispose(); }
});
