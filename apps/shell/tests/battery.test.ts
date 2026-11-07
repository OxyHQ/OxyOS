import { test } from "node:test";
import assert from "node:assert/strict";
import { shellMessages } from "../src/messages.ts";
import { batteryMenuLines, significantApps } from "../src/controls.ts";

const en = shellMessages("en-US");
const es = shellMessages("es-ES");

test("the battery's menu says its charge, the power source and the time left, as a Mac's", () => {
  const discharging = { battery: "discharging", percent: 64, onAC: false, minutes: 200 } as const;
  assert.deepEqual(batteryMenuLines(discharging, en), {
    charge: "64%",
    source: "Power Source: Battery",
    time: "3 h 20 min remaining",
  });
  assert.deepEqual(batteryMenuLines(discharging, es), {
    charge: "64 %",
    source: "Fuente de alimentación: batería",
    time: "Quedan 3 h 20 min",
  });
  const charging = { battery: "charging", percent: 78, onAC: true, minutes: 45 } as const;
  assert.equal(batteryMenuLines(charging, en)?.time, "45 min until full");
  assert.equal(batteryMenuLines(charging, en)?.source, "Power Source: Power Adapter");
  // Before UPower knows: calculating.
  assert.equal(batteryMenuLines({ ...charging, minutes: 0 }, en)?.time, "Calculating…");
  assert.equal(batteryMenuLines({ ...discharging, minutes: 0 }, es)?.time, "Calculando…");
  assert.equal(batteryMenuLines({ ...charging, battery: "full" }, en)?.time, "Fully Charged");
  // Held at its charge limit, plugged in.
  assert.equal(batteryMenuLines({ ...charging, battery: "not-charging" }, en)?.time, "Not Charging");
  // No battery: no menu.
  assert.equal(batteryMenuLines({ ...charging, battery: "absent" }, en), undefined);
  assert.equal(batteryMenuLines(undefined, en), undefined);
});

test("only the applications the session found using significant energy are listed, in its order", () => {
  const apps = [
    { id: "so.oxy.allo", name: "Allo", significant: true },
    { id: "so.oxy.inbox", name: "Inbox", significant: false },
    { id: "so.oxy.dropsea", name: "Dropsea", significant: true },
  ];
  assert.deepEqual(
    significantApps(apps).map((app) => app.name),
    ["Allo", "Dropsea"],
  );
  assert.deepEqual(significantApps([]), []);
});
