import { invoke } from "../../lib/tauri";
import AppLauncher from "./AppLauncher";

export default function LauncherWindow() {
  return (
    <div className="h-screen w-screen overflow-hidden bg-transparent">
      <AppLauncher onClose={() => invoke("hide_launcher")} />
    </div>
  );
}
