import { useState, useCallback } from "react";
import { useTermStore } from "./store";

export default function TabBar() {
  const tabs = useTermStore((s) => s.tabs);
  const activeTabId = useTermStore((s) => s.activeTabId);
  const setActiveTab = useTermStore((s) => s.setActiveTab);
  const addTab = useTermStore((s) => s.addTab);
  const closeTab = useTermStore((s) => s.closeTab);
  const renameTab = useTermStore((s) => s.renameTab);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");

  const handleDoubleClick = useCallback(
    (tabId: string, title: string) => {
      setEditingId(tabId);
      setEditValue(title);
    },
    [],
  );

  const commitRename = useCallback(() => {
    if (editingId && editValue.trim()) {
      renameTab(editingId, editValue.trim());
    }
    setEditingId(null);
  }, [editingId, editValue, renameTab]);

  const handleCloseTab = useCallback(
    (e: React.MouseEvent, tabId: string) => {
      e.stopPropagation();
      if (tabs.length === 1) {
        // Last tab — close window
        if (window.__TAURI_INTERNALS__) {
          import("@tauri-apps/api/window").then(({ getCurrentWindow }) => {
            getCurrentWindow().close();
          });
        }
      } else {
        closeTab(tabId);
      }
    },
    [tabs.length, closeTab],
  );

  return (
    <div className="flex h-[32px] shrink-0 items-end gap-0 border-b border-white/8 px-1">
      {tabs.map((tab) => (
        <button
          key={tab.id}
          onClick={() => setActiveTab(tab.id)}
          onDoubleClick={() => handleDoubleClick(tab.id, tab.title)}
          className={`group flex h-[28px] min-w-[100px] max-w-[180px] cursor-pointer items-center gap-1 rounded-t-lg px-3 text-[11px] font-medium transition-colors ${
            tab.id === activeTabId
              ? "bg-white/10 text-white/90"
              : "text-white/40 hover:bg-white/5 hover:text-white/60"
          }`}
        >
          {editingId === tab.id ? (
            <input
              autoFocus
              value={editValue}
              onChange={(e) => setEditValue(e.target.value)}
              onBlur={commitRename}
              onKeyDown={(e) => {
                if (e.key === "Enter") commitRename();
                if (e.key === "Escape") setEditingId(null);
              }}
              className="min-w-0 flex-1 bg-transparent text-[11px] text-white outline-none"
              onClick={(e) => e.stopPropagation()}
            />
          ) : (
            <span className="min-w-0 flex-1 truncate">{tab.title}</span>
          )}
          <span
            onClick={(e) => handleCloseTab(e, tab.id)}
            className="flex h-4 w-4 shrink-0 items-center justify-center rounded opacity-0 transition-opacity hover:bg-white/15 group-hover:opacity-100"
          >
            <svg width="7" height="7" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
              <path d="M1 1l8 8M9 1l-8 8" />
            </svg>
          </span>
        </button>
      ))}

      {/* New tab button */}
      <button
        onClick={addTab}
        className="flex h-[28px] w-[28px] shrink-0 cursor-pointer items-center justify-center rounded-t-lg text-white/30 transition-colors hover:bg-white/5 hover:text-white/60"
        aria-label="New tab"
      >
        <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
          <path d="M6 1v10M1 6h10" />
        </svg>
      </button>
    </div>
  );
}
