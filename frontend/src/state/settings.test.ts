import { describe, expect, it } from "vitest";
import {
  SETTINGS,
  overridesFromEntries,
  settingValue,
  withSetting,
} from "./settings";

const DEFAULT_PRELUDE = SETTINGS.querydown_prelude.default;

describe("overridesFromEntries", () => {
  it("keeps known keys and drops ones this build doesn't have", () => {
    expect(
      overridesFromEntries([
        { key: "querydown_prelude", value: "#track.x = 1" },
        { key: "from_a_newer_build", value: "?" },
      ]),
    ).toEqual({ querydown_prelude: "#track.x = 1" });
  });
});

describe("settingValue", () => {
  it("falls back to the default when the user hasn't customized it", () => {
    expect(settingValue({}, "querydown_prelude")).toBe(DEFAULT_PRELUDE);
  });

  it("returns the customized value when there is one", () => {
    expect(
      settingValue({ querydown_prelude: "#track.x = 1" }, "querydown_prelude"),
    ).toBe("#track.x = 1");
  });

  it("treats a customization to the empty string as a real value", () => {
    expect(settingValue({ querydown_prelude: "" }, "querydown_prelude")).toBe(
      "",
    );
  });
});

describe("withSetting", () => {
  it("records a changed value as an override", () => {
    expect(withSetting({}, "querydown_prelude", "#track.x = 1")).toEqual({
      querydown_prelude: "#track.x = 1",
    });
  });

  it("drops the override when the value is back to the default", () => {
    const customized = { querydown_prelude: "#track.x = 1" };
    expect(
      withSetting(customized, "querydown_prelude", DEFAULT_PRELUDE),
    ).toEqual({});
  });

  it("leaves the input untouched", () => {
    const before = { querydown_prelude: "#track.x = 1" };
    withSetting(before, "querydown_prelude", DEFAULT_PRELUDE);
    expect(before).toEqual({ querydown_prelude: "#track.x = 1" });
  });
});
