/** @vitest-environment happy-dom */
import {beforeEach, expect, it} from "vitest";
import {createMockCodexCatalogVersions} from "./mockCodexCatalogVersions";

beforeEach(() => localStorage.clear());

it("starts with a current version and persists selection without changing the runtime", async () => {
  const gateway = createMockCodexCatalogVersions(localStorage);
  expect(await gateway.getVersions()).toEqual({runtimeVersion: "0.146.0", selectedVersion: "0.146.0", availableVersions: ["0.146.0"]});
  expect((await gateway.getVersions(true)).selectedVersion).toBe("0.146.0");
  await gateway.selectVersion("0.200.0");
  expect(await createMockCodexCatalogVersions(localStorage).getVersions()).toEqual({runtimeVersion: "0.146.0", selectedVersion: "0.200.0", availableVersions: ["0.200.0", "0.146.0"]});
  await expect(gateway.selectVersion("9.9.9")).rejects.toThrow("codex_version_invalid");
});
