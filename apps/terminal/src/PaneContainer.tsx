import { useCallback } from "react";
import { type PaneNode, useTermStore } from "./store";
import TerminalView from "./TerminalView";
import SplitDivider from "./SplitDivider";

interface PaneContainerProps {
  tabId: string;
  node: PaneNode;
}

export default function PaneContainer({ tabId, node }: PaneContainerProps) {
  const resizeSplit = useTermStore((s) => s.resizeSplit);
  const setActivePane = useTermStore((s) => s.setActivePane);
  const activeTab = useTermStore((s) => s.tabs.find((t) => t.id === tabId));
  const activePaneId = activeTab?.activePaneId;

  const handleResize = useCallback(
    (ratio: number) => {
      if (node.type === "split") {
        resizeSplit(tabId, node.id, ratio);
      }
    },
    [tabId, node, resizeSplit],
  );

  if (node.type === "leaf") {
    return (
      <div
        className={`relative min-h-0 min-w-0 flex-1 ${
          node.id === activePaneId ? "ring-1 ring-inset ring-[#0a84ff]/30" : ""
        }`}
        onClick={() => setActivePane(tabId, node.id)}
      >
        <TerminalView ptyId={node.ptyId} />
      </div>
    );
  }

  // Split node
  const [first, second] = node.children;
  const isHorizontal = node.direction === "h";

  return (
    <div className={`flex min-h-0 min-w-0 flex-1 ${isHorizontal ? "flex-row" : "flex-col"}`}>
      <div
        className="min-h-0 min-w-0"
        style={{ flex: `${node.ratio} 1 0%` }}
      >
        <PaneContainer tabId={tabId} node={first} />
      </div>
      <SplitDivider direction={node.direction} onResize={handleResize} />
      <div
        className="min-h-0 min-w-0"
        style={{ flex: `${1 - node.ratio} 1 0%` }}
      >
        <PaneContainer tabId={tabId} node={second} />
      </div>
    </div>
  );
}
