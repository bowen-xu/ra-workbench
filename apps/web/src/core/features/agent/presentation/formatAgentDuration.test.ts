import { expect, it } from "vitest";
import { formatAgentDuration } from "./formatAgentDuration";

it.each([
  [0, "0秒"], [59, "59秒"], [60, "1分0秒"], [192, "3分12秒"],
  [3600, "1时0分0秒"], [86400, "1日0时0分0秒"], [194200, "2日5时56分40秒"],
  [59.9, "59秒"], [-1, "0秒"], [NaN, "0秒"], [Infinity, "0秒"],
])("formats %s seconds as %s", (seconds, expected) => {
  expect(formatAgentDuration(seconds)).toBe(expected);
});
