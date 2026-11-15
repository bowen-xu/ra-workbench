// @vitest-environment happy-dom
import {afterEach, expect, it, vi} from "vitest";
import {getAppRuntime, selectAppRuntime} from "../runtime/appRuntime";
import {agentTaskObservationGateway} from "./agentGateway";

const previous = getAppRuntime();
afterEach(() => selectAppRuntime(previous));

it("uses the production gateway through the native preview transport", async () => {
  const id = "11111111-1111-4111-8111-111111111111";
  const request = {conversationId: id, turnId: id, runId: id};
  const invoke = vi.fn(async () => ({schemaVersion: 1, ...request, status: {type: "unavailable"}}));
  selectAppRuntime({mode: "preview", storage: previous.storage, gateways: {}, invoke,
    nativeAgentPreview: {bootstrap: {schemaVersion: 1, generation: id, manuscriptId: id,
      model: {providerId: "openai", modelId: "synthetic", revision: null}, mechanism: "native-agent-raft-sqlite", supported: [], pending: []},
      invoke, events: {listen: async () => () => {}}, configure: async () => {}, reset: async () => {},
      reconnect() {}, subscribe: () => () => {}, getSnapshot: () => ({error: null}), dispose() {}}, dispose() {}});
  expect((await agentTaskObservationGateway.observe(request)).status).toEqual({type: "unavailable"});
  expect(invoke).toHaveBeenCalledExactlyOnceWith("agent_observe_task", {request});
});
