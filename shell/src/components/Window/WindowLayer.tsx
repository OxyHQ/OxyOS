import { AnimatePresence } from "framer-motion";
import { useWindowStore } from "../../stores/windowStore";
import AppWindow from "./AppWindow";
import TerminalApp from "../Terminal/TerminalApp";

const APP_COMPONENTS: Record<string, React.ComponentType<{ windowId: string }>> = {
  terminal: TerminalApp,
};

export default function WindowLayer() {
  const windows = useWindowStore((s) => s.windows);

  return (
    <AnimatePresence>
      {windows.map((win) => {
        const AppComponent = APP_COMPONENTS[win.appId];
        if (!AppComponent) return null;

        return (
          <AppWindow key={win.id} win={win}>
            <AppComponent windowId={win.id} />
          </AppWindow>
        );
      })}
    </AnimatePresence>
  );
}
