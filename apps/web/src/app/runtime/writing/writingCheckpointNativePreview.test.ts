// @vitest-environment happy-dom
import {afterEach, expect, it} from "vitest";
import {getAppRuntime, selectAppRuntime} from "../../../core/runtime/appRuntime";
import {getWritingCheckpointGateway} from "@research-assistant/writing-projects-plugin/writingCheckpointGateway";

const previous = getAppRuntime();
afterEach(() => selectAppRuntime(previous));

it("keeps an ordinary unconfigured preview closed instead of borrowing native checkpoint access", () => {
  selectAppRuntime({ mode: "preview", storage: window.localStorage, gateways: {}, dispose() {} });
  expect(() => getWritingCheckpointGateway()).toThrow("预览未覆盖业务网关");
});
