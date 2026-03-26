import { useState, useEffect } from "react";
import { invoke } from "../lib/tauri";

export interface InstalledApp {
  name: string;
  exec: string;
  icon: string;
  categories: string;
}

export function useInstalledApps() {
  const [apps, setApps] = useState<InstalledApp[]>([]);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    async function fetch() {
      const result = await invoke<InstalledApp[]>("list_desktop_apps");
      if (result && result.length > 0) {
        setApps(result);
      }
      setLoaded(true);
    }
    fetch();
  }, []);

  return { apps, loaded };
}
