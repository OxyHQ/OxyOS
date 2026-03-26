import { create } from "zustand";

// ── Pane tree types ──

export interface Leaf {
  type: "leaf";
  id: string;
  ptyId: string;
}

export interface Split {
  type: "split";
  id: string;
  direction: "h" | "v";
  children: [PaneNode, PaneNode];
  ratio: number;
}

export type PaneNode = Leaf | Split;

export interface Tab {
  id: string;
  title: string;
  root: PaneNode;
  activePaneId: string;
}

// ── Helpers ──

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

function collectPtyIds(node: PaneNode): string[] {
  if (node.type === "leaf") return [node.ptyId];
  return [...collectPtyIds(node.children[0]), ...collectPtyIds(node.children[1])];
}

function firstLeaf(node: PaneNode): Leaf {
  if (node.type === "leaf") return node;
  return firstLeaf(node.children[0]);
}

function cloneTree(node: PaneNode): PaneNode {
  return JSON.parse(JSON.stringify(node)) as PaneNode;
}

// ── Store ──

interface TermStore {
  tabs: Tab[];
  activeTabId: string;

  addTab: () => void;
  closeTab: (tabId: string) => void;
  setActiveTab: (tabId: string) => void;
  renameTab: (tabId: string, title: string) => void;

  splitPane: (tabId: string, paneId: string, direction: "h" | "v") => void;
  closePane: (tabId: string, paneId: string) => void;
  setActivePane: (tabId: string, paneId: string) => void;
  resizeSplit: (tabId: string, splitId: string, ratio: number) => void;

  getActiveTab: () => Tab | undefined;
  getPtyIdsForTab: (tabId: string) => string[];
}

const initialLeaf = makeLeaf();
const initialTab: Tab = {
  id: crypto.randomUUID(),
  title: "Terminal",
  root: initialLeaf,
  activePaneId: initialLeaf.id,
};

export const useTermStore = create<TermStore>((set, get) => ({
  tabs: [initialTab],
  activeTabId: initialTab.id,

  addTab: () => {
    const leaf = makeLeaf();
    const tab: Tab = {
      id: crypto.randomUUID(),
      title: "Terminal",
      root: leaf,
      activePaneId: leaf.id,
    };
    set((s) => ({
      tabs: [...s.tabs, tab],
      activeTabId: tab.id,
    }));
  },

  closeTab: (tabId) => {
    set((s) => {
      const remaining = s.tabs.filter((t) => t.id !== tabId);
      if (remaining.length === 0) return s; // don't close last tab here — App handles window close
      const activeTabId =
        s.activeTabId === tabId
          ? remaining[Math.max(0, s.tabs.findIndex((t) => t.id === tabId) - 1)]?.id ?? remaining[0]!.id
          : s.activeTabId;
      return { tabs: remaining, activeTabId };
    });
  },

  setActiveTab: (tabId) => set({ activeTabId: tabId }),

  renameTab: (tabId, title) =>
    set((s) => ({
      tabs: s.tabs.map((t) => (t.id === tabId ? { ...t, title } : t)),
    })),

  splitPane: (tabId, paneId, direction) =>
    set((s) => ({
      tabs: s.tabs.map((tab) => {
        if (tab.id !== tabId) return tab;
        const root = cloneTree(tab.root);
        const target = findNode(root, paneId);
        if (!target || target.type !== "leaf") return tab;

        const newLeaf = makeLeaf();
        const parent = findParent(root, paneId);

        const splitNode: Split = {
          type: "split",
          id: crypto.randomUUID(),
          direction,
          children: [{ ...target }, newLeaf],
          ratio: 0.5,
        };

        if (!parent) {
          // Target is root
          return { ...tab, root: splitNode, activePaneId: newLeaf.id };
        }

        parent.parent.children[parent.index] = splitNode;
        return { ...tab, root, activePaneId: newLeaf.id };
      }),
    })),

  closePane: (tabId, paneId) =>
    set((s) => ({
      tabs: s.tabs.map((tab) => {
        if (tab.id !== tabId) return tab;
        const root = cloneTree(tab.root);

        // If root is the pane being closed, can't close it (it's the last one)
        if (root.id === paneId) return tab;

        const result = findParent(root, paneId);
        if (!result) return tab;

        const sibling = result.parent.children[result.index === 0 ? 1 : 0];
        const grandparent = findParent(root, result.parent.id);

        if (!grandparent) {
          // Parent is root — replace root with sibling
          return { ...tab, root: sibling, activePaneId: firstLeaf(sibling).id };
        }

        grandparent.parent.children[grandparent.index] = sibling;
        return { ...tab, root, activePaneId: firstLeaf(sibling).id };
      }),
    })),

  setActivePane: (tabId, paneId) =>
    set((s) => ({
      tabs: s.tabs.map((t) => (t.id === tabId ? { ...t, activePaneId: paneId } : t)),
    })),

  resizeSplit: (tabId, splitId, ratio) =>
    set((s) => ({
      tabs: s.tabs.map((tab) => {
        if (tab.id !== tabId) return tab;
        const root = cloneTree(tab.root);
        const node = findNode(root, splitId);
        if (node?.type === "split") {
          node.ratio = Math.max(0.15, Math.min(0.85, ratio));
        }
        return { ...tab, root };
      }),
    })),

  getActiveTab: () => {
    const s = get();
    return s.tabs.find((t) => t.id === s.activeTabId);
  },

  getPtyIdsForTab: (tabId) => {
    const tab = get().tabs.find((t) => t.id === tabId);
    return tab ? collectPtyIds(tab.root) : [];
  },
}));
