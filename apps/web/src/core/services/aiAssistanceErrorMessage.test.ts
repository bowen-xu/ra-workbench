// @vitest-environment happy-dom
import { expect, it } from "vitest";
import { aiAssistanceErrorMessage } from "./aiAssistanceGateway";
it("keeps structured native error codes without displaying arbitrary diagnostic properties",()=>{
  expect(aiAssistanceErrorMessage({code:"authentication",message:"认证失败",privateResponse:"not for display"})).toBe("认证失败（错误码：authentication）");
  expect(aiAssistanceErrorMessage(Object.assign(new Error("读取失败"),{code:"storage_error"}))).toBe("读取失败（错误码：storage_error）");
  expect(aiAssistanceErrorMessage({message:"认证失败",code:"unsafe\nvalue"})).toBe("认证失败");
  expect(aiAssistanceErrorMessage(new Error("原有错误"))).toBe("原有错误");
});
