// @vitest-environment happy-dom
import {beforeEach, describe, expect, it, vi} from "vitest";
import type {ConfigurationSetting} from "../contracts/configuration";
import {ConfigurationError, createPluginConfigurations, type ConfigurationRegistration} from "./configuration";

const setting: ConfigurationSetting<boolean> = {key: "visible", defaultValue: true, schema: {type: "boolean"}, scopes: ["application", "resource"], effect: "immediate"};
const registration: ConfigurationRegistration = {owner: "fixture", declaration: {owner: "fixture", settings: [setting]}, storage: {visible: {application: "existing-visible", resourcePrefix: "existing-visible:"}}};
const forResource = (resourceId: string) => ({kind: "resource", resourceId} as const);
function open(registrations: readonly ConfigurationRegistration[] = [registration]) {
  const configuration = createPluginConfigurations(localStorage, registrations).get("fixture");
  if (!configuration) throw new Error("Fixture configuration was not bound");
  return configuration;
}
beforeEach(() => localStorage.clear());

describe("App-bound plugin configuration", () => {
  it("uses existing keys with default < application < resource priority and reopens the same facts", () => {
    const configuration = open();
    expect(configuration.inspect("visible")).toMatchObject({value: true, origin: "default", invalidScopes: []});
    configuration.update("visible", false);
    expect(localStorage.getItem("existing-visible")).toBe("false");
    expect(configuration.inspect("visible", forResource("one"))).toMatchObject({applicationValue: false, value: false, origin: "application"});
    configuration.update("visible", true, forResource("one"));
    expect(configuration.get("visible", forResource("one"))).toBe(true);
    expect(configuration.get("visible", forResource("two"))).toBe(false);
    expect(open().inspect("visible", forResource("one"))).toMatchObject({resourceValue: true, applicationValue: false, value: true, origin: "resource"});
    configuration.update("visible", undefined, forResource("one"));
    expect(configuration.get("visible", forResource("one"))).toBe(false);
    configuration.update("visible", undefined);
    expect(configuration.get("visible", forResource("one"))).toBe(true);
    expect(localStorage.length).toBe(0);
  });

  it("rejects foreign or unknown keys and invalid values without changing stored facts", () => {
    const configuration = open();
    configuration.update("visible", false);
    for (const value of ["true", null, {}, 1]) expect(() => configuration.update("visible", value)).toThrow("configuration_value_invalid");
    expect(() => configuration.get("other.visible")).toThrow("configuration_not_authorized");
    expect(() => configuration.update("other.visible", true)).toThrow("configuration_not_authorized");
    expect(localStorage.getItem("existing-visible")).toBe("false");
    expect(localStorage.length).toBe(1);
  });

  it("rejects unsupported scopes and invalid resource identities", () => {
    const configuration = open();
    for (const resourceId of ["", "bad\nidentity"]) expect(() => configuration.get("visible", forResource(resourceId))).toThrow("configuration_scope_invalid");
    const applicationOnly = {...registration, declaration: {owner: "fixture", settings: [{...setting, scopes: ["application"] as const}]}, storage: {visible: {application: "existing-visible"}}};
    expect(() => open([applicationOnly]).update("visible", true, forResource("one"))).toThrow("configuration_scope_invalid");
    expect(localStorage.length).toBe(0);
  });

  it("retains invalid legacy bytes and reports which override was ignored", () => {
    localStorage.setItem("existing-visible", "false");
    localStorage.setItem("existing-visible:one", '{"credential":"synthetic-not-a-secret"}');
    const configuration = open();
    expect(configuration.inspect("visible", forResource("one"))).toMatchObject({value: false, origin: "application", invalidScopes: ["resource"]});
    expect(localStorage.getItem("existing-visible:one")).toBe('{"credential":"synthetic-not-a-secret"}');
    expect(() => configuration.update("visible", "synthetic-not-a-secret")).toThrow("configuration_value_invalid");
    try { configuration.update("visible", "synthetic-not-a-secret"); } catch (reason) {
      expect(reason).toBeInstanceOf(ConfigurationError);
      expect(String(reason)).not.toContain("synthetic-not-a-secret");
    }
  });

  it("returns immutable detached values and keeps schema/defaults fixed after registration", () => {
    const schema = {type: "object" as const, properties: {order: {type: "string" as const, enum: ["first", "last"]}}};
    const defaultValue = {order: "first"};
    const objectRegistration = {owner: "fixture", declaration: {owner: "fixture", settings: [{key: "order", schema, defaultValue, scopes: ["application"] as const, effect: "immediate" as const}]}, storage: {order: {application: "existing-order"}}};
    const configuration = open([objectRegistration]);
    defaultValue.order = "last"; schema.properties.order.enum.push("injected");
    const value = configuration.get<{order: string}>("order");
    expect(value).toEqual({order: "first"});
    expect(Object.isFrozen(value)).toBe(true);
    expect(() => { value.order = "last"; }).toThrow();
    expect(() => configuration.update("order", {order: "injected"})).toThrow("configuration_value_invalid");
    const incoming = {order: "last"}, customEncoding = vi.fn(() => "different-unvalidated-value");
    Object.defineProperty(incoming, "toJSON", {value: customEncoding});
    configuration.update("order", incoming);
    expect(customEncoding).not.toHaveBeenCalled();
    expect(localStorage.getItem("existing-order")).toBe('{"order":"last"}');
    const inspection = configuration.inspect("order");
    expect(Object.isFrozen(inspection)).toBe(true);
    expect(Object.isFrozen(inspection.applicationValue)).toBe(true);
  });

  it("rejects owner, local-key and physical storage collisions rather than choosing a winner", () => {
    expect(() => createPluginConfigurations(localStorage, [registration, registration])).toThrow("configuration_owner_conflict");
    expect(() => open([{...registration, declaration: {owner: "fixture", settings: [setting, setting]}}])).toThrow("configuration_key_conflict");
    const other = {...registration, owner: "other", declaration: {owner: "other", settings: [setting]}};
    expect(() => createPluginConfigurations(localStorage, [registration, other])).toThrow("configuration_storage_conflict");
    expect(() => createPluginConfigurations(localStorage, [registration, {...other, storage: {visible: {application: "existing-visible:one", resourcePrefix: "other:"}}}])).toThrow("configuration_storage_conflict");
    expect(localStorage.length).toBe(0);
  });

  it("validates schema and default before publishing a bound owner", () => {
    expect(() => open([{...registration, owner: "another"}])).toThrow("configuration_owner_mismatch");
    expect(() => open([{...registration, declaration: {owner: "fixture", settings: [{...setting, defaultValue: "invalid"}]}}])).toThrow("configuration_declaration_invalid");
    const invalidEnum = {...setting, schema: {type: "string" as const, enum: ["same", "same"]}, defaultValue: "same"};
    expect(() => open([{...registration, declaration: {owner: "fixture", settings: [invalidEnum]}}])).toThrow("configuration_declaration_invalid");
  });

  it("notifies independent consumers only after changes and disposes only its own subscription", () => {
    const configuration = open(), first = vi.fn(), second = vi.fn();
    const leave = configuration.subscribe("visible", first), leaveOther = configuration.subscribe("visible", second);
    configuration.update("visible", false); configuration.update("visible", false);
    expect(first).toHaveBeenCalledTimes(1); expect(second).toHaveBeenCalledTimes(1);
    leave(); configuration.update("visible", true, forResource("one"));
    expect(first).toHaveBeenCalledTimes(1); expect(second).toHaveBeenCalledTimes(2);
    localStorage.setItem("existing-visible", "true");
    window.dispatchEvent(new StorageEvent("storage", {key: "existing-visible", newValue: "true"}));
    expect(second).toHaveBeenCalledTimes(3);
    leaveOther(); window.dispatchEvent(new StorageEvent("storage", {key: "existing-visible", newValue: "false"}));
    expect(second).toHaveBeenCalledTimes(3);
  });

  it("preserves the old value and a safe stable diagnostic when storage rejects an update", () => {
    localStorage.setItem("existing-visible", "false");
    const quota = new DOMException("synthetic-private-backend-detail", "QuotaExceededError");
    // happy-dom omits this legacy numeric field; browsers can supply it.
    Object.defineProperty(quota, "code", {value: 22});
    const failureStorage: Storage = {
      get length() { return localStorage.length; }, key: index => localStorage.key(index), getItem: key => localStorage.getItem(key),
      setItem: () => {throw quota;}, removeItem: () => {throw quota;}, clear: () => {},
    };
    const configuration = createPluginConfigurations(failureStorage, [registration]).get("fixture");
    if (!configuration) throw new Error("Fixture owner missing");
    const changed = vi.fn(), leave = configuration.subscribe("visible", changed);
    try {
      expect(() => configuration.update("visible", true)).toThrow("configuration_write_failed");
      expect(configuration.get("visible")).toBe(false);
      expect(changed).not.toHaveBeenCalled();
      try { configuration.update("visible", true); } catch (reason) {
        expect(String(reason)).toContain("QuotaExceededError");
        expect(reason instanceof ConfigurationError ? reason.safeCause?.name : undefined).toBe("QuotaExceededError");
        expect(reason instanceof ConfigurationError ? reason.safeCause?.code : undefined).toBe(22);
        expect(String(reason)).not.toContain("synthetic-private-backend-detail");
      }
    } finally { leave(); }
  });

  it("retains a standard storage permission cause without exposing the underlying read message", () => {
    const denied: Storage = {length: 0, key: () => null, clear: () => {}, removeItem: () => {}, setItem: () => {},
      getItem: () => {throw new DOMException("synthetic-private-read-message", "SecurityError");}};
    const configuration = createPluginConfigurations(denied, [registration]).get("fixture");
    if (!configuration) throw new Error("Fixture owner missing");
    try { configuration.get("visible"); throw new Error("Expected storage refusal"); } catch (reason) {
      expect(reason).toBeInstanceOf(ConfigurationError);
      expect(String(reason)).toContain("configuration_read_failed");
      expect(String(reason)).toContain("SecurityError");
      expect(String(reason)).not.toContain("synthetic-private-read-message");
    }
  });
});
