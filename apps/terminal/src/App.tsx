import { useCallback, useEffect } from "react";
import { useTermStore } from "./store";
import TabBar from "./TabBar";
import PaneContainer from "./PaneContainer";

export default function App() {
  const activeTab = useTermStore((s) => s.tabs.find((t) => t.id === s.activeTabId));
  const tabs = useTermStore((s) => s.tabs);
  const activeTabId = useTermStore((s) => s.activeTabId);
  const addTab = useTermStore((s) => s.addTab);
  const closePane = useTermStore((s) => s.closePane);
  const splitPane = useTermStore((s) => s.splitPane);
  const setActiveTab = useTermStore((s) => s.setActiveTab);

  const handleClose = useCallback(() => {
    if (window.__TAURI_INTERNALS__) {
      import("@tauri-apps/api/window").then(({ getCurrentWindow }) => {
        getCurrentWindow().close();
      });
    }
  }, []);

  const handleMinimize = useCallback(() => {
    if (window.__TAURI_INTERNALS__) {
      import("@tauri-apps/api/window").then(({ getCurrentWindow }) => {
        getCurrentWindow().minimize();
      });
    }
  }, []);

  const handleMaximize = useCallback(() => {
    if (window.__TAURI_INTERNALS__) {
      import("@tauri-apps/api/window").then(({ getCurrentWindow }) => {
        getCurrentWindow().toggleMaximize();
      });
    }
  }, []);

  // Keyboard shortcuts
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (e.ctrlKey && e.shiftKey) {
        switch (e.key) {
          case "T":
            e.preventDefault();
            addTab();
            break;
          case "W":
            e.preventDefault();
            if (activeTab) {
              if (activeTab.root.type === "leaf") {
                // Last pane in tab — close tab or window
                if (tabs.length === 1) {
                  handleClose();
                } else {
                  useTermStore.getState().closeTab(activeTab.id);
                }
              } else {
                closePane(activeTab.id, activeTab.activePaneId);
              }
            }
            break;
          case "H":
            e.preventDefault();
            if (activeTab) splitPane(activeTab.id, activeTab.activePaneId, "h");
            break;
          case "V":
            e.preventDefault();
            if (activeTab) splitPane(activeTab.id, activeTab.activePaneId, "v");
            break;
        }
      }

      // Ctrl+Tab / Ctrl+Shift+Tab for tab switching
      if (e.ctrlKey && e.key === "Tab") {
        e.preventDefault();
        const idx = tabs.findIndex((t) => t.id === activeTabId);
        if (e.shiftKey) {
          const prev = (idx - 1 + tabs.length) % tabs.length;
          setActiveTab(tabs[prev]!.id);
        } else {
          const next = (idx + 1) % tabs.length;
          setActiveTab(tabs[next]!.id);
        }
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [activeTab, tabs, activeTabId, addTab, closePane, splitPane, setActiveTab, handleClose]);

  return (
    <div className="flex h-screen w-screen flex-col overflow-hidden rounded-xl border border-white/15 bg-black/60 shadow-[0_12px_40px_rgba(0,0,0,0.5),inset_0_0.5px_0_rgba(255,255,255,0.08)] backdrop-blur-[60px] backdrop-saturate-[180%]">
      {/* Custom title bar */}
      <div
        data-tauri-drag-region
        className="flex h-[38px] shrink-0 items-center justify-between px-3"
      >
        {/* Traffic light buttons */}
        <div className="flex items-center gap-2">
          <button
            onClick={handleClose}
            className="h-3 w-3 cursor-pointer rounded-full bg-[#ff5f57] transition-opacity hover:opacity-80"
            aria-label="Close"
          />
          <button
            onClick={handleMinimize}
            className="h-3 w-3 cursor-pointer rounded-full bg-[#febc2e] transition-opacity hover:opacity-80"
            aria-label="Minimize"
          />
          <button
            onClick={handleMaximize}
            className="h-3 w-3 cursor-pointer rounded-full bg-[#28c840] transition-opacity hover:opacity-80"
            aria-label="Maximize"
          />
        </div>

        {/* Title */}
        <span className="pointer-events-none text-[12px] font-medium text-white/50">
          OxTerm
        </span>

        {/* Spacer */}
        <div className="w-[52px]" />
      </div>

      {/* Tab bar */}
      <TabBar />

      {/* Pane content */}
      <div className="flex min-h-0 flex-1">
        {activeTab && (
          <PaneContainer tabId={activeTab.id} node={activeTab.root} />
        )}
      </div>
    </div>
  );
}
