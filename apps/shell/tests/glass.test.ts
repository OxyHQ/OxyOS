import { test } from "node:test";
import assert from "node:assert/strict";
import { glass } from "../src/components/glass.ts";

test("glass is drawn as it is, and opaque with Reduce transparency on", () => {
  assert.equal(glass("#eef1f6eb", false), "#eef1f6eb");
  assert.equal(glass("#eef1f6eb", true), "#eef1f6");
  assert.equal(glass("#1B1F27B8", true), "#1b1f27");
  assert.equal(glass("#ffffff", true), "#ffffff");
  assert.equal(glass("rgba(255, 255, 255, 0.24)", true), "rgb(255, 255, 255)");
  // What it does not know is left as it is.
  assert.equal(glass("transparent", true), "transparent");
});
