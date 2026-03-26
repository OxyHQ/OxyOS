import { invoke } from "./tauri";
import { useWindowStore } from "../stores/windowStore";
import { useRunningAppsStore } from "../stores/runningAppsStore";

const BUILTIN_APPS: Record<string, { appId: string; title: string }> = {
  terminal: { appId: "terminal", title: "Terminal" },
};

export function launchApp(appName: string, exec: string) {
  if (exec.startsWith("__builtin:")) {
    const key = exec.replace("__builtin:", "");
    const builtin = BUILTIN_APPS[key];
    if (builtin) {
      useWindowStore.getState().openWindow(builtin.appId, builtin.title);
      useRunningAppsStore.getState().launch(appName);
    }
    return;
  }

  if (exec.startsWith("__internal:")) {
    return; // handled separately by caller
  }

  invoke("launch_app", { exec });
  useRunningAppsStore.getState().launch(appName);
}
