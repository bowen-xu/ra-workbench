import {expect, test} from "@playwright/test";
import {installCheckpointMetrics, seedCheckpointV3, verifyCheckpointUpgrade} from "./writing-checkpoints.browser";

test("v3 checkpoints upgrade atomically and stay out of actual App startup and hot navigation", async ({page}) => {
  test.setTimeout(90_000);
  const prefix = "research-assistant.app-preview.v3:";
  const fixture = await seedCheckpointV3(page, {prefix, count: 1024});
  expect(await verifyCheckpointUpgrade(page, prefix)).toEqual({errorCode: "", legacy: false, preference: "keep-preference", revision: 7, count: 2});
  await installCheckpointMetrics(page);
  await page.goto("/?preview=app");
  await page.getByRole("button", {name: "笔记", exact: true}).waitFor();
  expect(await page.evaluate(() => window.__checkpointMetrics)).toEqual({requests: {}, rows: 0, puts: 0});
  for (let i = 0; i < 5; i++) {
    await page.getByRole("button", {name: "笔记", exact: true}).click();
    await page.getByRole("button", {name: "文献", exact: true}).click();
  }
  expect(await page.evaluate(() => window.__checkpointMetrics)).toEqual({requests: {}, rows: 0, puts: 0});
  const results = await page.evaluate(async () => {
    const {getWritingCheckpointGateway} = await import("/plugins/ra-writing-projects/src/features/writing/gateway/writingCheckpointGateway.ts");
    const {getAppStorage} = await import("/ra-workbench/apps/web/src/core/runtime/appStorage.ts");
    const gateway = getWritingCheckpointGateway();
    const rows = await gateway.list("checkpoint-project-5");
    const listed = structuredClone(window.__checkpointMetrics);
    const saved = await gateway.save({expectedRevision: rows[0].revision, checkpoint: {...rows[0].checkpoint, version: 4, content: "new candidate"}});
    const committed = structuredClone(window.__checkpointMetrics);
    if (getAppStorage().getItem("writing-checkpoints") !== null) throw Error("legacy KV recreated");
    const read = await gateway.list("checkpoint-project-5");
    if (read.find(row => row.checkpoint.relativePath === saved.checkpoint.relativePath)?.checkpoint.content !== "new candidate") throw Error("missing saved receipt");
    return {listed, committed, savedRevision: saved.revision};
  });
  expect(results.listed).toEqual({requests: {"index.openCursor": 1}, rows: 2, puts: 0});
  expect(results.committed).toEqual({requests: {"index.openCursor": 1, "store.get": 1}, rows: 3, puts: 1});
  expect(results.savedRevision).toBe(8);
  await page.reload(); await page.getByRole("button", {name: "笔记", exact: true}).waitFor();
  expect(await page.evaluate(() => window.__checkpointMetrics)).toEqual({requests: {}, rows: 0, puts: 0});
  console.log("writing-checkpoint-records", JSON.stringify({fixture, startup: 0, hotTrips: 5, ...results}));
});

test("invalid checkpoint identity preserves the complete v3 bundle", async ({page}) => {
  const prefix = "checkpoint-corrupt-" + crypto.randomUUID();
  await seedCheckpointV3(page, {prefix, count: 12, invalid: true});
  expect(await verifyCheckpointUpgrade(page, prefix)).toEqual({errorCode: "writing_checkpoint_upgrade_failed", legacy: true, version: 3, hasStore: false});
});

test("checkpoint copy failure rolls back the version/store and permits a full retry", async ({page}) => {
  const prefix = "checkpoint-copy-failed-" + crypto.randomUUID();
  await seedCheckpointV3(page, {prefix, count: 12});
  expect(await verifyCheckpointUpgrade(page, prefix, true)).toEqual({errorCode: "writing_checkpoint_upgrade_failed", legacy: true, version: 3, hasStore: false});
  expect(await verifyCheckpointUpgrade(page, prefix)).toEqual({errorCode: "", legacy: false, preference: "keep-preference", revision: 7, count: 2});
});

test("v1 and v2 upgrades compose with the unchanged writing/catalog upgrades", async ({page}) => {
  for (const version of [1, 2] as const) {
    const prefix = `checkpoint-v${version}-${crypto.randomUUID()}`;
    await seedCheckpointV3(page, {prefix, count: 12, version});
    expect(await verifyCheckpointUpgrade(page, prefix)).toEqual({errorCode: "", legacy: false, preference: "keep-preference", revision: 7, count: 2});
  }
});

test("real checkpoint/file CAS domains are independent and reset clears the selected space", async ({page}) => {
  const prefix = "checkpoint-reset-" + crypto.randomUUID();
  await seedCheckpointV3(page, {prefix, count: 12});
  const result = await page.evaluate(async prefix => {
    const {createIndexedPreviewStorage} = await import("/ra-workbench/apps/web/src/core/runtime/previewIndexedStorage.ts");
    const {storagePersistence} = await import("/ra-workbench/apps/web/src/core/runtime/storagePersistence.ts");
    const {writingRecords} = await import("/ra-workbench/apps/web/src/core/public/writingStorage.ts");
    const {createBrowserWritingCheckpointGateway} = await import("/plugins/ra-writing-projects/src/features/writing/gateway/browserWritingCheckpointGateway.ts");
    const storage = await createIndexedPreviewStorage(prefix), records = writingRecords(storage), gateway = createBrowserWritingCheckpointGateway(storage);
    const formal = await records.read([]), rows = await gateway.list("checkpoint-project-5");
    await gateway.save({expectedRevision: 7, checkpoint: {...rows[0].checkpoint, version: 4, content: "changed"}});
    await records.commit(formal.revision, [{store: "writingSessions", key: "unrelated", value: {}}]);
    const other = await createIndexedPreviewStorage(prefix + "other"), otherGateway = createBrowserWritingCheckpointGateway(other);
    await otherGateway.save({expectedRevision: null, checkpoint: rows[0].checkpoint});
    await storagePersistence(storage).reset();
    let closed = false;
    try { await gateway.list("checkpoint-project-5"); } catch (reason) { closed = reason.code === "writing_storage_closed"; }
    storagePersistence(storage).close(); await storagePersistence(storage).flush();
    const reopened = await createIndexedPreviewStorage(prefix);
    try {
      const empty = await createBrowserWritingCheckpointGateway(reopened).list("checkpoint-project-5");
      const untouched = await otherGateway.list("checkpoint-project-5");
      return {closed, resetRows: empty.length, otherRows: untouched.length, originalRevision: formal.revision};
    } finally {
      storagePersistence(reopened).close(); await storagePersistence(reopened).flush();
      storagePersistence(other).close(); await storagePersistence(other).flush();
    }
  }, prefix);
  expect(result).toEqual({closed: true, resetRows: 0, otherRows: 1, originalRevision: 12});
});
