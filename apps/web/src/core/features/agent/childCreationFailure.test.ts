import { expect, it } from "vitest";
import { childCreationFailureStage } from "./childCreationFailure";

it("matches cleanup only to the saved child and submission and a consistent archive intent", () => {
  const child = {childId: "11111111-1111-4111-8111-111111111111", operationId: "22222222-2222-4222-8222-222222222222", submissionRequestId: "33333333-3333-4333-8333-333333333333"};
  const archive = "44444444-4444-5444-8444-444444444444";
  const detail = {code: "DATA_SQL_COMMIT_UNKNOWN", requestId: child.submissionRequestId, resultId: child.childId,
    childArchiveIntentVersion: 1, childArchiveOperationId: archive, journalRequestId: archive, childArchiveExpectedWatermark: 0};
  const classify = (value: object) => childCreationFailureStage(`tool_outcome_unknown: ${JSON.stringify(value)}`, child, "parent", "turn");
  expect(classify(detail)).toBe("cleanup");
  for (const bad of [{...detail, requestId: child.operationId}, {...detail, resultId: child.operationId},
    {...detail, journalRequestId: child.operationId}, {...detail, childArchiveIntentVersion: 2},
    {...detail, childArchiveExpectedWatermark: -1}, {...detail, code: "OTHER"}, {...detail, turnId: "other"}]) {
    expect(classify(bad)).toBeUndefined();
  }
  expect(childCreationFailureStage(`tool_outcome_unknown: ${JSON.stringify(detail)}`, {childId: child.childId, operationId: child.operationId}, "parent", "turn")).toBeUndefined();
});
