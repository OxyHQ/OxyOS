import { test } from "node:test";
import assert from "node:assert/strict";
import { shellMessages } from "../../shell/src/messages.ts";
import { isPowerKey, LOGIN_POWER_ACTIONS, loginPowerLabel, loginPowerRefusal, loginPowerRequest } from "../src/power.ts";

const en = shellMessages("en-US");
const es = shellMessages("es-ES");

test("the login screen's foot is a Mac's login window's: Sleep, Restart, Shut Down", () => {
  assert.deepEqual(
    LOGIN_POWER_ACTIONS.map((action) => loginPowerLabel(action, en)),
    ["Sleep", "Restart", "Shut Down"],
  );
  assert.deepEqual(
    LOGIN_POWER_ACTIONS.map((action) => loginPowerLabel(action, es)),
    ["Reposo", "Reiniciar", "Apagar"],
  );
});

test("each acts at once: nobody is signed in, so nothing is confirmed", () => {
  assert.deepEqual(loginPowerRequest("sleep"), { sleep: {} });
  assert.deepEqual(loginPowerRequest("restart"), { endSession: { action: "restart", confirm: false } });
  assert.deepEqual(loginPowerRequest("shutDown"), { endSession: { action: "shutDown", confirm: false } });
});

test("a refusal is said with why; what is happening says nothing", () => {
  assert.equal(loginPowerRefusal("shutDown", { ending: { action: "shutDown" } }, en), null);
  assert.equal(loginPowerRefusal("sleep", { sleeping: {} }, en), null);
  assert.equal(
    loginPowerRefusal("shutDown", { refused: { reason: "notPermitted" } }, en),
    "Your computer can't shut down now. Someone else is logged in to this computer, or an application is keeping it on. An administrator must shut it down.",
  );
  assert.equal(
    loginPowerRefusal("restart", { failed: { reason: "unavailable" } }, es),
    "El ordenador no puede reiniciarse ahora. OxyOS no ha podido conectar con el servicio de energía del sistema.",
  );
});

test("the power key is told unasked as {powerKey: {}}", () => {
  assert.ok(isPowerKey({ powerKey: {} }));
  assert.ok(!isPowerKey({ network: {} }));
  assert.ok(!isPowerKey(null));
});
