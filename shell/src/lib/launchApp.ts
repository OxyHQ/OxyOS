import { invoke } from "./tauri";
import { useRunningAppsStore } from "../stores/runningAppsStore";

export function launchApp(appName: string, exec: string) {
  if (exec.startsWith("__builtin:")) {
    const key = exec.replace("__builtin:", "");
    if (key === "terminal") {
      invoke("open_terminal");
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
