import { describe, expect, it } from "vitest";
import { advanceAuroraSequence, appendStableBlocksDelta } from "./auroraStream";

describe("Aurora stream protocol", () => {
  it("rejects duplicate and out-of-order frames", () => {
    expect(advanceAuroraSequence(7, 7)).toBeNull();
    expect(advanceAuroraSequence(7, 6)).toBeNull();
    expect(advanceAuroraSequence(7, 8)).toBe(8);
    expect(advanceAuroraSequence(7, 0)).toBe(7);
  });

  it("appends consecutive stable block deltas without replaying history", () => {
    const first = appendStableBlocksDelta(
      [],
      [{ hash: "a", content: "first" }],
    );
    const second = appendStableBlocksDelta(first, [
      { hash: "b", content: "second" },
      { hash: "c", content: "third" },
    ]);

    expect(second.map((block) => block.hash)).toEqual(["a", "b", "c"]);
    expect(first.map((block) => block.hash)).toEqual(["a"]);
  });
});
