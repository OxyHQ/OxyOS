import React, {
  useRef,
  useMemo,
  useEffect,
  useCallback,
  useState,
} from "react";
import {
  Image,
  useWindowDimensions,
  type ScrollView,
  type View,
} from "react-native";
import { Gesture, GestureDetector } from "react-native-gesture-handler";
import Animated, { runOnJS } from "react-native-reanimated";
import {
  useOverviewDrag,
  type OverviewDragController,
} from "./useOverviewDrag";
import { nearestWorkspace } from "../overview-geometry";
import { Button } from "@oxy.so/bloom/button";
import { Text } from "@oxy.so/bloom/typography";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
} from "@oxy.so/bloom/dropdown-menu";
import { RiAddLine } from "@oxy.so/bloom/icons/RiAddLine";
import { RiCloseLine } from "@oxy.so/bloom/icons/RiCloseLine";
import { RiArrowDownSLine } from "@oxy.so/bloom/icons/RiArrowDownSLine";
import { RiWindowLine } from "@oxy.so/bloom/icons/RiWindowLine";
import { Box, PressArea, Scroll } from "../primitives";
import { AppIcon } from "../apps";
import type {
  ShellSnapshot,
  ShellActions,
  ShellWindow,
  ShellApplication,
} from "../model";

function aspectRatio(window: ShellWindow) {
  const size = window.thumbnailSize;
  return size &&
    Number.isFinite(size.width) &&
    Number.isFinite(size.height) &&
    size.width > 0 &&
    size.height > 0
    ? Math.max(0.45, Math.min(3, size.width / size.height))
    : 16 / 10;
}

function WindowThumbnail({
  window,
  app,
  contentHeight,
  focus,
  close,
  drag,
  faded = false,
}: {
  window: ShellWindow;
  app?: ShellApplication;
  contentHeight: number;
  focus: () => void;
  close: () => void;
  drag?: OverviewDragController;
  faded?: boolean;
}) {
  const width = contentHeight * aspectRatio(window);
  const id = window.id;
  const gesture = useMemo(
    () =>
      Gesture.Pan()
        .enabled(!!drag)
        .minDistance(8)
        .enableTrackpadTwoFingerGesture(false)
        .onStart((event) => {
          if (!drag) return;
          drag.pointerX.value = event.absoluteX;
          drag.pointerY.value = event.absoluteY;
          drag.grabX.value = event.x;
          drag.grabY.value = event.y;
          drag.active.value = true;
          runOnJS(drag.begin)(id, contentHeight);
        })
        .onUpdate((event) => {
          if (!drag?.active.value) return;
          drag.pointerX.value = event.absoluteX;
          drag.pointerY.value = event.absoluteY;
        })
        .onEnd((event, success) => {
          if (drag)
            runOnJS(drag.finish)(event.absoluteX, event.absoluteY, success);
        })
        .onFinalize((event, success) => {
          if (drag && !success)
            runOnJS(drag.finish)(event.absoluteX, event.absoluteY, false);
        }),
    [drag, id, contentHeight],
  );
  const content = (
    <Box
      testID={`overview-window-${window.id}`}
      className="overflow-hidden rounded-[13px] bg-[#eef3f8]"
      style={{
        width,
        opacity: faded ? 0.25 : 1,
        boxShadow: "0 1px 3px #172e4320",
      }}
    >
      <Box className="h-6 flex-row items-center bg-[#d0e5f3] pl-2 pr-1">
        <PressArea
          accessibilityRole="button"
          accessibilityLabel={`Ir a ${window.title}`}
          onPress={focus}
          className="min-w-0 flex-1 flex-row items-center gap-1.5"
        >
          {app && <AppIcon app={app} size={14} circular />}
          <Text
            numberOfLines={1}
            className="min-w-0 flex-1 text-[12px] font-normal text-[#526672]"
          >
            {window.title}
          </Text>
        </PressArea>
        <Button
          iconOnly
          icon={RiCloseLine}
          iconSize={15}
          accessibilityLabel={`Cerrar ${window.title}`}
          testID={`close-window-${window.id}`}
          appearance="plain"
          className="ml-1 h-5 w-5 shrink-0 rounded-full border-0 bg-transparent p-0"
          colors={{ background: "transparent", foreground: "#82909a" }}
          onPress={close}
        />
      </Box>
      <PressArea
        accessibilityRole="button"
        accessibilityLabel={`Activar ventana ${window.title}`}
        onPress={focus}
        className="items-center justify-center"
        style={{ width, height: contentHeight }}
      >
        {window.thumbnail ? (
          <Image
            source={window.thumbnail}
            resizeMode="contain"
            style={{ width, height: contentHeight }}
            accessibilityLabel={`Miniatura de ${window.title}`}
          />
        ) : (
          <Box className="items-center gap-3">
            {app && <AppIcon app={app} size={40} />}
            <Text className="text-xs text-[#657683]">
              Vista previa no disponible
            </Text>
          </Box>
        )}
      </PressArea>
    </Box>
  );
  return drag ? (
    <GestureDetector gesture={gesture} touchAction="pan-y">
      {content}
    </GestureDetector>
  ) : (
    content
  );
}

export function Overview({
  snapshot,
  actions,
  dismiss,
  escapeRequest = 0,
}: {
  snapshot: ShellSnapshot;
  actions: ShellActions;
  dismiss: () => void;
  escapeRequest?: number;
}) {
  const { width, height } = useWindowDimensions();
  const desktops = useRef<React.ComponentRef<typeof ScrollView>>(null);
  const positioned = useRef<string | null>(null);
  const stage = useRef<React.ComponentRef<typeof View>>(null);
  const [origin, setOrigin] = useState({ x: 0, y: 0 });
  const scrollOffset = useRef(0);
  const settleTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const scrollTo = useCallback(
    (x: number) => desktops.current?.scrollTo({ x, animated: false }),
    [],
  );
  const cardWidth = Math.round(width * (width < 700 ? 0.86 : 0.7));
  const cardHeight = Math.max(260, Math.round(height * 0.7));
  const gap = Math.round(width * 0.025);
  const sideInset = (width - cardWidth) / 2;
  const initialIndex = Math.max(
    0,
    snapshot.workspaces.findIndex(
      (workspace) => workspace.id === snapshot.workspaceId,
    ),
  );
  const drag = useOverviewDrag(
    snapshot,
    actions,
    {
      ...origin,
      width,
      cardWidth,
      cardHeight,
      gap,
      inset: sideInset,
      top: Math.round(height * 0.07),
      count: snapshot.workspaces.length,
    },
    scrollTo,
  );
  const latest = useRef({ snapshot, actions, dismiss });
  latest.current = { snapshot, actions, dismiss };
  const settle = () => {
    if (drag.activeWindow.current) return;
    const index = nearestWorkspace(
      scrollOffset.current,
      cardWidth + gap,
      latest.current.snapshot.workspaces.length,
    );
    const workspace = latest.current.snapshot.workspaces[index];
    if (!workspace) return;
    const offset = index * (cardWidth + gap);
    if (Math.abs(scrollOffset.current - offset) > 1)
      desktops.current?.scrollTo({ x: offset, animated: true });
    if (workspace.id !== latest.current.snapshot.workspaceId)
      latest.current.actions.activateWorkspace(workspace.id);
  };
  const settleRef = useRef(settle);
  settleRef.current = settle;
  useEffect(
    () => () => {
      if (settleTimer.current) clearTimeout(settleTimer.current);
    },
    [],
  );
  const previousEscape = useRef(escapeRequest);
  useEffect(() => {
    if (previousEscape.current !== escapeRequest && !drag.cancel())
      latest.current.dismiss();
    previousEscape.current = escapeRequest;
  }, [escapeRequest, drag.cancel]);
  const hadDrag = useRef(false);
  useEffect(() => {
    if (!drag.dragged && hadDrag.current) settleRef.current();
    hadDrag.current = !!drag.dragged;
  }, [drag.dragged]);
  const enterWorkspace = (id: string) => {
    actions.activateWorkspace(id);
    dismiss();
  };
  return (
    <Box
      ref={stage}
      onLayout={() =>
        stage.current?.measureInWindow((x, y) => setOrigin({ x, y }))
      }
      testID="shell-overview"
      className="absolute inset-0 bg-[#18334a]/10"
      style={{ zIndex: 20 }}
    >
      <PressArea
        accessibilityRole="button"
        accessibilityLabel="Cerrar vista de escritorios"
        onPress={dismiss}
        className="absolute inset-0"
      />
      <Scroll
        ref={desktops}
        testID="overview-desktops"
        onScroll={(event) => {
          scrollOffset.current = event.nativeEvent.contentOffset.x;
          drag.scroll.value = scrollOffset.current;
          if (settleTimer.current) clearTimeout(settleTimer.current);
          if (!drag.activeWindow.current)
            settleTimer.current = setTimeout(() => settleRef.current(), 160);
        }}
        scrollEventThrottle={16}
        horizontal
        showsHorizontalScrollIndicator={false}
        contentOffset={{ x: initialIndex * (cardWidth + gap), y: 0 }}
        onContentSizeChange={() => {
          const key = `${cardWidth}:${initialIndex}`;
          if (positioned.current === key) return;
          desktops.current?.scrollTo({
            x: initialIndex * (cardWidth + gap),
            animated: false,
          });
          positioned.current = key;
          scrollOffset.current = initialIndex * (cardWidth + gap);
          drag.scroll.value = scrollOffset.current;
        }}
        snapToInterval={cardWidth + gap}
        decelerationRate="fast"
        contentContainerStyle={{
          paddingLeft: sideInset,
          paddingRight: sideInset,
          paddingTop: Math.round(height * 0.07),
          gap,
        }}
        className="flex-1"
      >
        {snapshot.workspaces.map((workspace) => {
          const windows = snapshot.windows.filter(
            (window) => window.workspaceId === workspace.id,
          );
          const columns = cardWidth >= 650 ? 3 : cardWidth >= 420 ? 2 : 1;
          const rows = Array.from(
            { length: Math.ceil(windows.length / columns) },
            (_, index) => windows.slice(index * columns, (index + 1) * columns),
          );
          const availableHeight = cardHeight - 116;
          const rowHeight = Math.max(
            104,
            Math.min(
              310,
              (availableHeight - Math.max(0, rows.length - 1) * 16) /
                Math.max(1, rows.length),
            ),
          );
          return (
            <Box
              key={workspace.id}
              testID={`workspace-${workspace.id}`}
              className="overflow-hidden rounded-[24px] bg-[#546776]/35"
              style={{
                width: cardWidth,
                height: cardHeight,
                boxShadow:
                  drag.dragged &&
                  snapshot.workspaces[drag.target]?.id === workspace.id &&
                  drag.dragged.window.workspaceId !== workspace.id
                    ? "inset 0 0 0 2px #ffffffc0"
                    : undefined,
              }}
            >
              <PressArea
                accessibilityRole="button"
                accessibilityLabel={`Abrir ${workspace.name}`}
                onPress={() => enterWorkspace(workspace.id)}
                className="absolute inset-0"
              />
              <Box className="absolute left-3 top-3" style={{ zIndex: 2 }}>
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <Button
                      appearance="plain"
                      leadingIcon={RiWindowLine}
                      trailingIcon={RiArrowDownSLine}
                      iconSize={15}
                      className="h-8 rounded-full border-0 bg-[#f7f9fc] px-3"
                      colors={{ background: "#f7f9fc", foreground: "#3f4d58" }}
                      textStyle={{ fontSize: 12, fontWeight: "500" }}
                      accessibilityLabel={`Opciones de ${workspace.name}`}
                    >
                      {workspace.name}
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent
                    label={workspace.name}
                    size="sm"
                    minWidth={200}
                    className="rounded-xl"
                  >
                    <DropdownMenuItem
                      onPress={() => enterWorkspace(workspace.id)}
                    >
                      Abrir este escritorio
                    </DropdownMenuItem>
                    {snapshot.workspaces
                      .filter((other) => other.id !== workspace.id)
                      .map((other) => (
                        <DropdownMenuItem
                          key={other.id}
                          onPress={() =>
                            desktops.current?.scrollTo({
                              x:
                                snapshot.workspaces.findIndex(
                                  (item) => item.id === other.id,
                                ) *
                                (cardWidth + gap),
                              animated: true,
                            })
                          }
                        >
                          Ver {other.name.toLocaleLowerCase()}
                        </DropdownMenuItem>
                      ))}
                  </DropdownMenuContent>
                </DropdownMenu>
              </Box>
              <Scroll
                className="mt-20 mb-9 flex-1"
                contentContainerClassName="gap-4 px-4"
                showsVerticalScrollIndicator={false}
              >
                {rows.map((row, index) => {
                  const ratioSum = row.reduce(
                    (sum, window) => sum + aspectRatio(window),
                    0,
                  );
                  // One scale per row: preserve surface proportions and fit the available width.
                  const contentHeight = Math.min(
                    rowHeight - 24,
                    (cardWidth - 32 - (row.length - 1) * 16) / ratioSum,
                  );
                  return (
                    <Box
                      key={index}
                      className={
                        windows.length === 1
                          ? "flex-row items-start gap-4"
                          : "flex-row items-start justify-center gap-4"
                      }
                      style={
                        windows.length === 1
                          ? { paddingLeft: Math.round(cardWidth * 0.06) }
                          : undefined
                      }
                    >
                      {row.map((window) => (
                        <WindowThumbnail
                          key={window.id}
                          window={window}
                          drag={drag.controller}
                          faded={drag.dragged?.window.id === window.id}
                          app={snapshot.applications.find(
                            (app) => app.id === window.appId,
                          )}
                          contentHeight={contentHeight}
                          close={() => {
                            if (drag.canPress()) actions.closeWindow(window.id);
                          }}
                          focus={() => {
                            if (!drag.canPress()) return;
                            actions.activateWorkspace(workspace.id);
                            actions.focusWindow(window.id);
                            dismiss();
                          }}
                        />
                      ))}
                    </Box>
                  );
                })}
                {!windows.length && (
                  <PressArea
                    accessibilityRole="button"
                    accessibilityLabel={`Abrir ${workspace.name} vacío`}
                    onPress={() => enterWorkspace(workspace.id)}
                    className="items-center justify-center"
                    style={{ height: availableHeight }}
                  >
                    <Text className="text-[13px] text-white/75">
                      No hay ventanas abiertas
                    </Text>
                  </PressArea>
                )}
              </Scroll>
            </Box>
          );
        })}
        <Box
          className="items-center justify-center"
          style={{ height: cardHeight, width: 160 }}
        >
          <Button
            iconOnly
            icon={RiAddLine}
            iconSize={25}
            accessibilityLabel="Añadir escritorio"
            appearance="plain"
            className="h-16 w-16 rounded-full border border-white/25 bg-white/20 p-0"
            colors={{ background: "#ffffff30", foreground: "#fff" }}
            onPress={actions.addWorkspace}
          />
          <Text className="mt-3 text-xs text-white/90">Nuevo escritorio</Text>
        </Box>
      </Scroll>
      {drag.dragged && (
        <Animated.View
          testID="overview-drag-preview"
          pointerEvents="none"
          style={[
            {
              position: "absolute",
              left: 0,
              top: 0,
              zIndex: 100,
              boxShadow: "0 18px 50px #10243f55",
              borderRadius: 13,
            },
            drag.style,
          ]}
        >
          <WindowThumbnail
            window={drag.dragged.window}
            app={snapshot.applications.find(
              (app) => app.id === drag.dragged!.window.appId,
            )}
            contentHeight={drag.dragged.contentHeight}
            focus={() => {}}
            close={() => {}}
          />
        </Animated.View>
      )}
    </Box>
  );
}
