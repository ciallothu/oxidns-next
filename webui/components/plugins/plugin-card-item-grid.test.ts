import { describe, expect, it } from "vitest";

import { pluginCardGridColumnClass } from "./plugin-card-item-grid";

describe("plugin card item grid", () => {
  it.each([1, 2, 3])("uses one column for %i items", (itemCount) => {
    expect(pluginCardGridColumnClass(itemCount)).toBe("grid-cols-1");
  });

  it.each([4, 5, 6])("uses two columns for %i items", (itemCount) => {
    expect(pluginCardGridColumnClass(itemCount)).toBe("grid-cols-2");
  });
});
