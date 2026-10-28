import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  useAnimatedReaction,
  useAnimatedStyle,
  useSharedValue,
  runOnJS,
} from "react-native-reanimated";
import type { ShellActions, ShellSnapshot, ShellWindow } from "../model";
import { workspaceAtPoint, type OverviewGeometry } from "../overview-geometry";

export function useOverviewDrag(
  snapshot: ShellSnapshot,
  actions: ShellActions,
  geometry: OverviewGeometry,
  scrollTo: (x: number) => void,
) {
  const latest = useRef({ snapshot, actions, scrollTo });
  latest.current = { snapshot, actions, scrollTo };
  const [dragged, setDragged] = useState<{
    window: ShellWindow;
    contentHeight: number;
  } | null>(null);
  const [target, setTarget] = useState(-1);
  const activeWindow = useRef<string | null>(null);
  const suppressUntil = useRef(0);
  const frame = useRef<number | null>(null);
  const edge = useRef(0);
  const mounted = useRef(true);
  const active = useSharedValue(false);
  const pointerX = useSharedValue(0);
  const pointerY = useSharedValue(0);
  const grabX = useSharedValue(0);
  const grabY = useSharedValue(0);
  const scroll = useSharedValue(0);
  const metrics = useSharedValue(geometry);
  useEffect(() => {
    metrics.value = geometry;
  }, [
    geometry.x,
    geometry.y,
    geometry.width,
    geometry.cardWidth,
    geometry.cardHeight,
    geometry.gap,
    geometry.inset,
    geometry.top,
    geometry.count,
  ]);

  const stopEdge = useCallback(() => {
    edge.current = 0;
    if (frame.current !== null) cancelAnimationFrame(frame.current);
    frame.current = null;
  }, []);
  const setEdge = useCallback(
    (direction: number) => {
      stopEdge();
      edge.current = direction;
      if (!direction) return;
      let previous = 0;
      const advance = (time: number) => {
        if (!activeWindow.current || !edge.current) return;
        const g = metrics.value;
        const elapsed = previous ? Math.min(32, time - previous) : 16;
        previous = time;
        const maximum = Math.max(0, (g.count - 1) * (g.cardWidth + g.gap));
        const next = Math.max(
          0,
          Math.min(maximum, scroll.value + edge.current * elapsed * 0.85),
        );
        scroll.value = next;
        latest.current.scrollTo(next);
        frame.current = requestAnimationFrame(advance);
      };
      frame.current = requestAnimationFrame(advance);
    },
    [metrics, scroll, stopEdge],
  );
  const updateTarget = useCallback((index: number) => {
    if (mounted.current) setTarget(index);
  }, []);
  useAnimatedReaction(
    () => {
      if (!active.value) return { target: -1, edge: 0 };
      const g = metrics.value;
      const x = pointerX.value - g.x;
      return {
        target: workspaceAtPoint(
          pointerX.value,
          pointerY.value,
          scroll.value,
          g,
        ),
        edge: x < 72 ? -1 : x > g.width - 72 ? 1 : 0,
      };
    },
    (value, previous) => {
      if (value.target !== previous?.target)
        runOnJS(updateTarget)(value.target);
      if (value.edge !== previous?.edge) runOnJS(setEdge)(value.edge);
    },
    [active, pointerX, pointerY, metrics, scroll, setEdge, updateTarget],
  );
  const cancel = useCallback(() => {
    const wasDragging = activeWindow.current !== null;
    activeWindow.current = null;
    active.value = false;
    stopEdge();
    if (wasDragging) suppressUntil.current = Date.now() + 300;
    if (mounted.current) {
      setDragged(null);
      setTarget(-1);
    }
    return wasDragging;
  }, [active, stopEdge]);
  const begin = useCallback((id: string, contentHeight: number) => {
    const window = latest.current.snapshot.windows.find(
      (item) => item.id === id,
    );
    if (!window || !mounted.current) return;
    activeWindow.current = id;
    suppressUntil.current = Infinity;
    setDragged({ window, contentHeight });
  }, []);
  const finish = useCallback(
    (x: number, y: number, success: boolean) => {
      const current = latest.current;
      const id = activeWindow.current;
      const index = workspaceAtPoint(x, y, scroll.value, metrics.value);
      const destination = current.snapshot.workspaces[index];
      const window = current.snapshot.windows.find((item) => item.id === id);
      cancel();
      if (
        success &&
        window &&
        destination &&
        window.workspaceId !== destination.id
      )
        current.actions.moveWindowToWorkspace(window.id, destination.id);
    },
    [cancel, scroll, metrics],
  );
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      activeWindow.current = null;
      active.value = false;
      stopEdge();
    };
  }, [active, stopEdge]);
  useEffect(() => {
    if (
      activeWindow.current &&
      !snapshot.windows.some((window) => window.id === activeWindow.current)
    )
      cancel();
  }, [snapshot.windows, cancel]);
  const style = useAnimatedStyle(
    () => ({
      transform: [
        { translateX: pointerX.value - metrics.value.x - grabX.value },
        { translateY: pointerY.value - metrics.value.y - grabY.value },
        { scale: 1.025 },
      ],
    }),
    [pointerX, pointerY, grabX, grabY, metrics],
  );
  const controller = useMemo(
    () => ({ active, pointerX, pointerY, grabX, grabY, begin, finish }),
    [active, pointerX, pointerY, grabX, grabY, begin, finish],
  );
  return {
    controller,
    dragged,
    target,
    style,
    scroll,
    cancel,
    activeWindow,
    canPress: () => Date.now() >= suppressUntil.current,
  };
}
export type OverviewDragController = ReturnType<
  typeof useOverviewDrag
>["controller"];
