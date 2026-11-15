// @vitest-environment happy-dom
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {getAppRuntime, selectAppRuntime, type AppRuntime} from "../runtime/appRuntime";
import {createTestNamespacedStorage} from "../runtime/testing/createTestNamespacedStorage";
import {getLiteratureGateway, getWritingGateway} from "./sourceDomainGateways";

let previous: AppRuntime, owner: AppRuntime;
beforeEach(() => {
  previous = {...getAppRuntime(), storage: getAppRuntime().storage};
  owner = {mode: "browser", storage: createTestNamespacedStorage(window.localStorage, "source-domain-test:"), gateways: {}, dispose() {}};
  selectAppRuntime(owner);
});
afterEach(() => {owner.storage.clear(); selectAppRuntime(previous);});

it("keeps optional source inventories empty when no domain provider is installed", async () => {
  await expect(getLiteratureGateway().list()).resolves.toEqual([]);
  await expect(getWritingGateway().listManuscripts()).resolves.toEqual([]);
  expect(getAppRuntime()).toBe(owner);
  expect(owner.gateways).toEqual({});
});

it("rejects explicit source reads with the stable unavailable code instead of inventing an object", async () => {
  await expect(getLiteratureGateway().get("paper-7")).rejects.toMatchObject({
    code: "source_domain_unavailable", message: expect.stringContaining("文献能力"),
  });
  await expect(getWritingGateway().listFiles("manuscript-9")).rejects.toMatchObject({
    code: "source_domain_unavailable", message: expect.stringContaining("写作能力"),
  });
  expect(owner.gateways).toEqual({});
});

it("uses providers installed on the same owner even when the source facade was obtained earlier", async () => {
  const literature = getLiteratureGateway(), writing = getWritingGateway();
  const get = vi.fn(async () => ({id: "paper-7"})), list = vi.fn(async () => []);
  const listManuscripts = vi.fn(async () => []), listFiles = vi.fn(async () => []);
  owner.gateways.getLiteratureGateway = {get, list} as unknown as NonNullable<AppRuntime["gateways"]["getLiteratureGateway"]>;
  owner.gateways.getWritingGateway = {listManuscripts, listFiles} as unknown as NonNullable<AppRuntime["gateways"]["getWritingGateway"]>;
  await expect(literature.get("paper-7")).resolves.toEqual({id: "paper-7"});
  await expect(literature.list()).resolves.toEqual([]);
  await expect(writing.listManuscripts()).resolves.toEqual([]);
  await expect(writing.listFiles("manuscript-9")).resolves.toEqual([]);
  expect(get).toHaveBeenCalledExactlyOnceWith("paper-7");
  expect(listFiles).toHaveBeenCalledExactlyOnceWith("manuscript-9");
  expect(getAppRuntime()).toBe(owner);
});

it("preserves a provided domain's failure and diagnostics for inventories and explicit reads", async () => {
  const failure = {code: "source_read_failed", requestId: "request-7", operationId: "operation-9", status: 503, message: "safe original cause"};
  const reject = vi.fn(async () => {throw failure;});
  owner.gateways.getLiteratureGateway = {get: reject, list: reject} as unknown as NonNullable<AppRuntime["gateways"]["getLiteratureGateway"]>;
  owner.gateways.getWritingGateway = {listManuscripts: reject, listFiles: reject} as unknown as NonNullable<AppRuntime["gateways"]["getWritingGateway"]>;
  for (const read of [
    () => getLiteratureGateway().list(), () => getLiteratureGateway().get("paper-7"),
    () => getWritingGateway().listManuscripts(), () => getWritingGateway().listFiles("manuscript-9"),
  ]) await expect(read()).rejects.toBe(failure);
  expect(reject).toHaveBeenCalledTimes(4);
});
