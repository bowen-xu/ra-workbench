import {expect, it} from "vitest";
import {decodeAgentWritingPrepareRequest} from "@research-assistant/core/agent";
import {writingPreparationRejection} from "./writingPreparationRejection";
const c = "11111111-1111-4111-8111-111111111111", m = "22222222-2222-4222-8222-222222222222", r = "33333333-3333-4333-8333-333333333333";
const request = decodeAgentWritingPrepareRequest({conversationId: c, requestId: r, text: "original", context: {manuscriptId: m, files: []}});
const allowed = [
  ["DATA_SQL_OPERATION_CONFLICT", "writing-prepare-materialize", "writing_context_changed"],
  ["DATA_SQL_OPERATION_CONFLICT", "writing-prepare-materialize", "writing_input_changed"],
  ["DATA_SQL_OPERATION_CONFLICT", "writing-prepare-materialize", "writing_instructions_unauthorized"],
  ...["writing_binding_unavailable", "writing_context_invalid", "writing_context_budget_exceeded", "writing_input_invalid", "writing_instructions_scope_invalid", "writing_instructions_budget_exceeded", "writing_instructions_encoding_invalid"]
    .map(cause => ["DATA_SQL_INVALID", "writing-prepare-materialize", cause]),
  ["DATA_SQL_INVALID", "writing-prepare-scope", "writing_context_scope_invalid"],
];
function failure(code = "DATA_SQL_OPERATION_CONFLICT", phase = "writing-prepare-materialize", safeCause = "writing_context_changed") {
  return {code: "storage", reason: {conversationId: c, manuscriptId: m, requestId: r, operationId: r, code,
    reason: {operationId: r, epoch: null, sqlFence: null, phase, sqliteCode: null, sqliteExtendedCode: null, safeCause}}};
}
it.each(allowed)("accepts only proven fresh rollback %s/%s/%s and retains its safe diagnostic", (code, phase, cause) => {
  const original = failure(code, phase, cause);
  expect(writingPreparationRejection(original, request)).toEqual(original);
  expect(writingPreparationRejection({...original, message: "PRIVATE_BODY", reason: {...original.reason, message: "PRIVATE_SQL"}}, request)).toEqual(original);
});
it("does not treat wrong identities, phase, cause, Unknown codes or arbitrary bodies as rejection proof", () => {
  const original = failure();
  const invalid = [failure("DATA_SQL_COMMIT_UNKNOWN"), failure("DATA_SQL_UNAVAILABLE"), failure(undefined, "commit"), failure(undefined, undefined, "arbitrary_failure"),
    failure(undefined, undefined, "operation_id_content_changed"),
    ...["conversationId", "manuscriptId", "requestId", "operationId"].map(field => ({...original, reason: {...original.reason, [field]: field === "conversationId" ? m : c}})),
    {...original, reason: {...original.reason, reason: {...original.reason.reason, operationId: c}}}];
  for (const value of invalid) expect(writingPreparationRejection(value, request)).toBeNull();
  const missing = structuredClone(original); delete (missing.reason.reason as Partial<typeof missing.reason.reason>).epoch;
  expect(writingPreparationRejection(missing, request)).toBeNull();
});
