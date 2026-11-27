import { test } from "node:test";
import assert from "node:assert/strict";
import { messagesFor } from "../src/messages.ts";
import { readFileSync } from "node:fs";
import { ABOUT_WINDOW, aboutThisComputer, MORE_INFO_URL, type SystemFacts } from "../src/model.ts";

const SYSTEM: SystemFacts = {
  computerName: "ThinkPad P14s",
  version: "OxyOS (development)",
  processor: "12th Gen Intel Core i7-1270P",
  memoryBytes: 32 * 1024 ** 3,
  display: "1920 × 1200",
  timeZone: "Europe/Madrid",
  automaticTime: true,
  keyboardLayout: "us",
  locale: "es-ES",
};

test("About This Computer says Settings › About's facts, in a Mac's order: processor, memory, OxyOS", () => {
  assert.deepEqual(aboutThisComputer(SYSTEM, messagesFor("en-US")), {
    name: "ThinkPad P14s",
    rows: [
      { key: "processor", label: "Processor", value: "12th Gen Intel Core i7-1270P" },
      { key: "memory", label: "Memory", value: "32 GB" },
      { key: "version", label: "OxyOS", value: "OxyOS (development)" },
    ],
    moreInfo: "More Info…",
  });
  const es = aboutThisComputer(SYSTEM, messagesFor("es-ES"));
  assert.deepEqual(
    es.rows.map((row) => row.label),
    ["Procesador", "Memoria", "OxyOS"],
  );
  assert.equal(es.moreInfo, "Más información…");
  // More Info… opens Settings at its About page, as a Mac's opens General › About.
  assert.equal(MORE_INFO_URL, "oxyos-settings://about");
});

test("a fact the system does not tell is left out, never guessed", () => {
  const about = aboutThisComputer(
    { ...SYSTEM, computerName: " ", processor: "", memoryBytes: 0 },
    messagesFor("en-US"),
  );
  assert.equal(about.name, "");
  assert.deepEqual(
    about.rows.map((row) => row.key),
    ["version"],
  );
});

test("About This Computer's package declares the window size its limits keep, so it opens at it", () => {
  const description = JSON.parse(
    readFileSync(new URL("../../../tools/rn-host-spike/sample/packages/so.oxy.aboutthiscomputer.json", import.meta.url), "utf8"),
  );
  assert.deepEqual(description.window, { width: ABOUT_WINDOW.width, height: ABOUT_WINDOW.height });
});
