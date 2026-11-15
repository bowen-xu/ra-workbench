import {expect, it, vi} from "vitest";
import {TextRecoveryCheckpoints} from "./TextRecoveryCheckpoints";
const initial = {content: "candidate", baseline: "before", saved: "candidate", savedBaseline: "before"};
function storage() {
  const values = new Map<string, string>();
  return {values, port: {getItem: vi.fn((key: string) => values.get(key) ?? null), setItem: vi.fn((key: string, value: string) => {values.set(key, value);})} as unknown as Storage};
}
it("serializes an unchanged snapshot once, but preserves baseline-only and saved-version changes", () => {
  const {port} = storage(), cache = new TextRecoveryCheckpoints(() => port), serialize = vi.spyOn(JSON, "stringify");
  try {
    for(let visit = 0; visit < 20; visit++) cache.write("draft", {...initial});
    expect(serialize).toHaveBeenCalledOnce(); expect(port.setItem).toHaveBeenCalledOnce();
    cache.write("draft", {...initial, baseline: "candidate"}); expect(port.setItem).toHaveBeenCalledTimes(2);
    cache.write("draft", {...initial, saved: "external", savedBaseline: "external"}); expect(port.setItem).toHaveBeenCalledTimes(3);
    expect(JSON.parse(port.getItem("draft")!)).toEqual({...initial, saved: "external", savedBaseline: "external"});
  } finally {serialize.mockRestore();}
});
it("rechecks deletion, overwrite and storage ownership instead of trusting the previous write", () => {
  const first = storage(), second = storage(); let owner = first.port;
  const cache = new TextRecoveryCheckpoints(() => owner); cache.write("draft", initial);
  first.values.delete("draft"); cache.write("draft", initial);
  first.values.set("draft", "another host"); cache.write("draft", initial);
  expect(first.port.setItem).toHaveBeenCalledTimes(3);
  owner = second.port; cache.write("draft", initial); expect(second.port.setItem).toHaveBeenCalledOnce();
});
it("propagates failed durable reads/writes and releases cached serializations of closed documents", () => {
  const {port} = storage(), cache = new TextRecoveryCheckpoints(() => port);
  vi.mocked(port.setItem).mockImplementationOnce(() => {throw new Error("quota");});
  expect(() => cache.write("draft", initial)).toThrow("quota"); cache.write("draft", initial);
  vi.mocked(port.getItem).mockImplementationOnce(() => {throw new Error("unavailable");});
  expect(() => cache.write("draft", initial)).toThrow("unavailable");
  cache.retain([]);
  const serialize = vi.spyOn(JSON, "stringify");
  try {cache.write("draft", initial); expect(serialize).toHaveBeenCalledOnce();}
  finally {serialize.mockRestore();}
});
