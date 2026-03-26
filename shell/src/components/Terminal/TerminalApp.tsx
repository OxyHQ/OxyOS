import { useState, useMemo, useCallback, useEffect } from "react";
import TerminalView from "./TerminalView";

// ── Pane tree types ──

interface Leaf {
  type: "leaf";
  id: string;
  ptyId: string;
}

interface Split {
  type: "split";
  id: string;
  direction: "h" | "v";
  children: [PaneNode, PaneNode];
  ratio: number;
}

type PaneNode = Leaf | Split;

interface Tab {
  id: string;
  title: string;
  root: PaneNode;
  activePaneId: string;
}

function makeLeaf(): Leaf {
  const id = crypto.randomUUID();
  return { type: "leaf", id, ptyId: id };
}

function findNode(node: PaneNode, id: string): PaneNode | null {
  if (node.id === id) return node;
  if (node.type === "split") {
    return findNode(node.children[0], id) ?? findNode(node.children[1], id);
  }
  return null;
}

function findParent(node: PaneNode, id: string): { parent: Split; index: 0 | 1 } | null {
  if (node.type === "split") {
    if (node.children[0].id === id) return { parent: node, index: 0 };
    if (node.children[1].id === id) return { parent: node, index: 1 };
    return findParent(node.children[0], id) ?? findParent(node.children[1], id);
  }
  return null;
}

function firstLeaf(node: PaneNode): Leaf {
  if (node.type === "leaf") return node;
  return firstLeaf(node.children[0]);
}

function cloneTree(node: PaneNode): PaneNode {
  return JSON.parse(JSON.stringify(node)) as PaneNode;
}

// ── Component ──

export default function TerminalApp({ windowId: _windowId }: { windowId: string }) {
  const initialLeaf = useMemo(() => makeLeaf(), []);
  const [tabs, setTabs] = useState<Tab[]>(() => [{
    id: crypto.randomUUID(),
    title: "Terminal",
    root: initialLeaf,
    activePaneId: initialLeaf.id,
  }]);
  const [activeTabId, setActiveTabId] = useState(tabs[0]!.id);

  const activeTab = tabs.find((t) => t.id === activeTabId);

  const addTab = useCallback(() => {
    const leaf = makeLeaf();
    const tab: Tab = { id: crypto.randomUUID(), title: "Terminal", root: leaf, activePaneId: leaf.id };
    setTabs((prev) => [...prev, tab]);
    setActiveTabId(tab.id);
  }, []);

  const closeTab = useCallback((tabId: string) => {
    setTabs((prev) => {
      const remaining = prev.filter((t) => t.id !== tabId);
      if (remaining.length === 0) return prev;
      return remaining;
    });
    setActiveTabId((prev) => {
      if (prev === tabId) {
        const idx = tabs.findIndex((t) => t.id === tabId);
        const remaining = tabs.filter((t) => t.id !== tabId);
        return remaining[Math.max(0, idx - 1)]?.id ?? remaining[0]?.id ?? prev;
      }
      return prev;
    });
  }, [tabs]);

  const splitPane = useCallback((tabId: string, paneId: string, direction: "h" | "v") => {
    setTabs((prev) => prev.map((tab) => {
      if (tab.id !== tabId) return tab;
      const root = cloneTree(tab.root);
      const target = findNode(root, paneId);
      if (!target || target.type !== "leaf") return tab;

      const newLeaf = makeLeaf();
      const splitNode: Split = {
        type: "split",
        id: crypto.randomUUID(),
        direction,
        children: [{ ...target }, newLeaf],
        ratio: 0.5,
      };

      const parent = findParent(root, paneId);
      if (!parent) {
        return { ...tab, root: splitNode, activePaneId: newLeaf.id };
      }
      parent.parent.children[parent.index] = splitNode;
      return { ...tab, root, activePaneId: newLeaf.id };
    }));
  }, []);

  const closePane = useCallback((tabId: string, paneId: string) => {
    setTabs((prev) => prev.map((tab) => {
      if (tab.id !== tabId) return tab;
      const root = cloneTree(tab.root);
      if (root.id === paneId) return tab;

      const result = findParent(root, paneId);
      if (!result) return tab;

      const sibling = result.parent.children[result.index === 0 ? 1 : 0];
      const grandparent = findParent(root, result.parent.id);

      if (!grandparent) {
        return { ...tab, root: sibling, activePaneId: firstLeaf(sibling).id };
      }
      grandparent.parent.children[grandparent.index] = sibling;
      return { ...tab, root, activePaneId: firstLeaf(sibling).id };
    }));
  }, []);

  const resizeSplit = useCallback((tabId: string, splitId: string, ratio: number) => {
    setTabs((prev) => prev.map((tab) => {
      if (tab.id !== tabId) return tab;
      const root = cloneTree(tab.root);
      const node = findNode(root, splitId);
      if (node?.type === "split") {
        node.ratio = Math.max(0.15, Math.min(0.85, ratio));
      }
      return { ...tab, root };
    }));
  }, []);

  const setActivePane = useCallback((tabId: string, paneId: string) => {
    setTabs((prev) => prev.map((t) => (t.id === tabId ? { ...t, activePaneId: paneId } : t)));
  }, []);

  // Keyboard shortcuts
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (e.ctrlKey && e.shiftKey) {
        switch (e.key) {
          case "T": e.preventDefault(); addTab(); break;
          case "W":
            e.preventDefault();
            if (activeTab) {
              if (activeTab.root.type === "leaf") {
                if (tabs.length > 1) closeTab(activeTab.id);
              } else {
                closePane(activeTab.id, activeTab.activePaneId);
              }
            }
            break;
          case "H": e.preventDefault(); if (activeTab) splitPane(activeTab.id, activeTab.activePaneId, "h"); break;
          case "V": e.preventDefault(); if (activeTab) splitPane(activeTab.id, activeTab.activePaneId, "v"); break;
        }
      }
      if (e.ctrlKey && e.key === "Tab") {
        e.preventDefault();
        const idx = tabs.findIndex((t) => t.id === activeTabId);
        const next = e.shiftKey
          ? (idx - 1 + tabs.length) % tabs.length
          : (idx + 1) % tabs.length;
        setActiveTabId(tabs[next]!.id);
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [activeTab, tabs, activeTabId, addTab, closeTab, closePane, splitPane]);

  return (
    <div className="flex h-full flex-col">
      {/* Tab bar */}
      <div className="flex h-[30px] shrink-0 items-end gap-0 border-b border-white/8 px-1">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            onClick={() => setActiveTabId(tab.id)}
            className={`group flex h-[26px] min-w-[90px] max-w-[160px] cursor-pointer items-center gap-1 rounded-t-lg px-2.5 text-[11px] font-medium transition-colors ${
              tab.id === activeTabId
                ? "bg-white/10 text-white/90"
                : "text-white/40 hover:bg-white/5 hover:text-white/60"
            }`}
          >
            <span className="min-w-0 flex-1 truncate">{tab.title}</span>
            {tabs.length > 1 && (
              <span
                onClick={(e) => { e.stopPropagation(); closeTab(tab.id); }}
                className="flex h-4 w-4 shrink-0 items-center justify-center rounded opacity-0 transition-opacity hover:bg-white/15 group-hover:opacity-100"
              >
                <svg width="6" height="6" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
                  <path d="M1 1l8 8M9 1l-8 8" />
                </svg>
              </span>
            )}
          </button>
        ))}
        <button
          onClick={addTab}
          className="flex h-[26px] w-[26px] shrink-0 cursor-pointer items-center justify-center rounded-t-lg text-white/30 transition-colors hover:bg-white/5 hover:text-white/60"
          aria-label="New tab"
        >
          <svg width="10" height="10" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
            <path d="M6 1v10M1 6h10" />
          </svg>
        </button>
      </div>

      {/* Pane content */}
      <div className="flex min-h-0 flex-1">
        {activeTab && (
          <PaneContainer
            tabId={activeTab.id}
            node={activeTab.root}
            activePaneId={activeTab.activePaneId}
            onSetActivePane={setActivePane}
            onResizeSplit={resizeSplit}
          />
        )}
      </div>
    </div>
  );
}

// ── Pane container (recursive) ──

function PaneContainer({
  tabId,
  node,
  activePaneId,
  onSetActivePane,
  onResizeSplit,
}: {
  tabId: string;
  node: PaneNode;
  activePaneId: string;
  onSetActivePane: (tabId: string, paneId: string) => void;
  onResizeSplit: (tabId: string, splitId: string, ratio: number) => void;
}) {
  if (node.type === "leaf") {
    return (
      <div
        className={`relative min-h-0 min-w-0 flex-1 ${
          node.id === activePaneId ? "ring-1 ring-inset ring-[#0a84ff]/30" : ""
        }`}
        onClick={() => onSetActivePane(tabId, node.id)}
      >
        <TerminalView ptyId={node.ptyId} />
      </div>
    );
  }

  const [first, second] = node.children;
  const isH = node.direction === "h";

  return (
    <div className={`flex min-h-0 min-w-0 flex-1 ${isH ? "flex-row" : "flex-col"}`}>
      <div className="min-h-0 min-w-0" style={{ flex: `${node.ratio} 1 0%` }}>
        <PaneContainer tabId={tabId} node={first} activePaneId={activePaneId} onSetActivePane={onSetActivePane} onResizeSplit={onResizeSplit} />
      </div>
      <SplitDivider direction={node.direction} onResize={(r) => onResizeSplit(tabId, node.id, r)} />
      <div className="min-h-0 min-w-0" style={{ flex: `${1 - node.ratio} 1 0%` }}>
        <PaneContainer tabId={tabId} node={second} activePaneId={activePaneId} onSetActivePane={onSetActivePane} onResizeSplit={onResizeSplit} />
      </div>
    </div>
  );
}

// ── Split divider ──

function SplitDivider({ direction, onResize }: { direction: "h" | "v"; onResize: (ratio: number) => void }) {
  const handlePointerDown = useCallback(
    (e: React.PointerEvent) => {
      e.preventDefault();
      const el = e.currentTarget;
      const parent = el.parentElement;
      if (!parent) return;

      const rect = parent.getBoundingClientRect();
      el.setPointerCapture(e.pointerId);

      const handleMove = (ev: PointerEvent) => {
        const ratio = direction === "h"
          ? (ev.clientX - rect.left) / rect.width
          : (ev.clientY - rect.top) / rect.height;
        onResize(ratio);
      };

      const handleUp = () => {
        el.removeEventListener("pointermove", handleMove);
        el.removeEventListener("pointerup", handleUp);
      };

      el.addEventListener("pointermove", handleMove);
      el.addEventListener("pointerup", handleUp);
    },
    [direction, onResize],
  );

  return (
    <div
      onPointerDown={handlePointerDown}
      className={`shrink-0 bg-white/8 transition-colors hover:bg-[#0a84ff]/50 ${
        direction === "h" ? "w-[4px] cursor-col-resize" : "h-[4px] cursor-row-resize"
      }`}
    />
  );
}
