/**
 * OxyOS's Activity Monitor (`so.oxy.activitymonitor`), as macOS's: a toolbar window whose table
 * lists the machine's processes in one of five views (CPU, Memory, Energy, Disk, Network), each
 * launched app's processes grouped under it, sorted by any column, searched and narrowed to the
 * person's own; under it the view's figures for the whole machine and their last three minutes.
 * A process the person owns can be quit (or force quit), any can be inspected.
 *
 * Pure: the platform is the `source` it is given (model.ts), which on OxyOS is the session's
 * `activity` vocabulary. Every change on screen comes from a sample the session tells while the
 * window shows; the app keeps no timer, follows the samples only while it is mounted, draws only
 * the table's visible rows, and draws again only the rows a sample changed.
 */
import * as React from "react";
import {
  FlatList,
  Image,
  Pressable,
  View,
  type LayoutChangeEvent,
  type ListRenderItemInfo,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
} from "react-native";
import { Button } from "@oxy.so/bloom/button";
import { Dialog } from "@oxy.so/bloom/dialog";
import { RiArrowRightSLine, RiCloseCircleLine, RiInformationLine, RiWindowLine } from "@oxy.so/bloom/icons";
import { useWindowKeys } from "@oxy.so/bloom/kbd";
import {
  Menubar,
  MenubarCheckboxItem,
  MenubarContent,
  MenubarItem,
  MenubarMenu,
  MenubarSeparator,
  MenubarTrigger,
} from "@oxy.so/bloom/menubar";
import { PageHeader } from "@oxy.so/bloom/page-header";
import { Screen } from "@oxy.so/bloom/screen";
import { Search } from "@oxy.so/bloom/search";
import { SegmentedControl, SegmentedControlItem, SegmentedControlItemText } from "@oxy.so/bloom/segmented-control";
import { Table, TableCell, TableColumn, TableHeader, TableRow } from "@oxy.so/bloom/table";
import { useTheme } from "@oxy.so/bloom/theme";
import { Text } from "@oxy.so/bloom/typography";
import { useWindowSizeLimits } from "@oxy.so/bloom/window";
import { Inspector, type InspectTarget } from "./Inspector";
import { messagesFor, type Messages } from "./messages";
import {
  canInspect,
  canQuit,
  COLUMNS,
  DEFAULT_SORTS,
  filterRows,
  flattenRows,
  groupRows,
  History,
  keepUnchanged,
  nextSort,
  pointOf,
  quitOutcome,
  rowView,
  sortRows,
  VIEWS,
  type ActivitySample,
  type ActivitySource,
  type ActivityView,
  type ColumnKey,
  type QuitOutcome,
  type Row,
  type RowView,
  type Sort,
} from "./model";
import { Summary } from "./Summary";

export type ActivityMonitorProps = {
  /** The platform's activity; null where there is none (a host outside a session). */
  source: ActivitySource | null;
  /** The session's locale (BCP 47), which picks the words and writes the numbers. */
  locale: string;
};

/** A body row of Bloom's `sm` table: 6 + 18 + 6, and the hairline under it. */
const ROW_HEIGHT = 31;
/** The disclosure triangle's and the icon's slots, so every name starts in the same place. */
const DISCLOSURE = 14;
const ICON = 16;
/** Two presses on a row this close are a double click: the inspector, as macOS's. */
const DOUBLE_PRESS_MS = 400;
/** How many samples a quit's failure stays under the table: about six seconds. */
const NOTICE_SAMPLES = 3;

/** Each column's width: the name takes what the fixed columns leave. */
const COLUMN_LAYOUT: Record<ColumnKey, { width?: number; flex?: number; minWidth?: number; align?: "start" | "end" }> = {
  name: { flex: 1, minWidth: 160 },
  cpu: { width: 76, align: "end" },
  cpuTime: { width: 104, align: "end" },
  threads: { width: 96, align: "end" },
  wakeups: { width: 132, align: "end" },
  gpu: { width: 76, align: "end" },
  pid: { width: 72, align: "end" },
  user: { width: 104 },
  memory: { width: 104, align: "end" },
  swap: { width: 104, align: "end" },
  energy: { width: 132, align: "end" },
  average: { width: 116, align: "end" },
  written: { width: 120, align: "end" },
  read: { width: 112, align: "end" },
  writeRate: { width: 112, align: "end" },
  readRate: { width: 108, align: "end" },
  received: { width: 128, align: "end" },
  sent: { width: 112, align: "end" },
  receivedRate: { width: 112, align: "end" },
  sentRate: { width: 104, align: "end" },
  packetsIn: { width: 116, align: "end" },
  packetsOut: { width: 120, align: "end" },
};

function columnLabel(key: ColumnKey, view: ActivityView, words: Messages): string {
  if (key === "name") return view === "energy" ? words.columns.appName : view === "network" ? words.columns.interface : words.columns.name;
  return words.columns[key];
}

/** The room a header's label takes in the table's `sm` type: an estimate of its advance, generous. */
const LABEL_ADVANCE = 7.8;
/** A sortable header's chevron (24) and its cell's insets (10 + 10), with a pixel to spare. */
const HEADER_CHROME = 46;

/**
 * A column's layout in a language: a fixed column at least as wide as its header, whose label and
 * sort chevron Bloom's table lays out in one row that does not shrink ("Intercambio" is wider than
 * "Swap"), so a header never runs into the next.
 */
function columnLayout(key: ColumnKey, label: string) {
  const layout = COLUMN_LAYOUT[key];
  if (layout.width === undefined) return layout;
  return { ...layout, width: Math.max(layout.width, Math.ceil(label.length * LABEL_ADVANCE) + HEADER_CHROME) };
}

/** The window's narrowest: the widest view's fixed columns beside the name's least, in the language. */
function narrowest(words: Messages): number {
  let widest = 0;
  for (const view of VIEWS) {
    let total = COLUMN_LAYOUT.name.minWidth ?? 0;
    for (const key of COLUMNS[view]) total += columnLayout(key, columnLabel(key, view, words)).width ?? 0;
    widest = Math.max(widest, total);
  }
  return widest;
}

/** An app's icon from its package, else a neutral window glyph. */
function AppIcon({ path, color }: { path: string | null; color: string }) {
  if (path) {
    const uri = path.startsWith("/") ? `file://${path}` : path;
    return <Image source={{ uri }} style={{ width: ICON, height: ICON }} accessibilityIgnoresInvertColors />;
  }
  return <RiWindowLine width={ICON} height={ICON} fill={color} />;
}

/** One row of the table, drawn again only when its view (its words) or its selection changes. */
const ActivityRow = React.memo(function ActivityRow({
  row,
  selected,
  slots,
  onPress,
  onToggle,
  words,
}: {
  row: RowView;
  selected: boolean;
  /** Whether names keep the disclosure's and the icon's room (every view but Network). */
  slots: boolean;
  onPress: (key: string) => void;
  onToggle: (key: string) => void;
  words: Messages;
}) {
  const theme = useTheme();
  const name = row.cells[0];
  return (
    // Not a focus stop of its own: the table's keys are the window's (↑ ↓ → ← Return), as a Mac's
    // table is one control, so no row keeps a focus ring the selection has moved away from.
    <Pressable accessible={false} focusable={false} onPress={() => onPress(row.key)}>
      <TableRow
        selected={selected}
        testID={row.testID}
        style={{ height: ROW_HEIGHT, opacity: row.resting ? 0.45 : 1 }}
      >
        {row.cells.map((text, index) =>
          index === 0 ? (
            <TableCell key="name">
              <View style={{ flexDirection: "row", alignItems: "center", gap: 6, paddingLeft: row.depth === 1 ? DISCLOSURE + 6 : 0 }}>
                {slots && row.depth === 0 ? (
                  row.expandable ? (
                    <Pressable
                      onPress={() => onToggle(row.key)}
                      hitSlop={8}
                      accessibilityRole="button"
                      accessibilityLabel={row.expanded ? words.collapse(name) : words.expand(name)}
                      accessibilityState={{ expanded: row.expanded }}
                      testID={`${row.testID}-disclosure`}
                      style={{ width: DISCLOSURE, alignItems: "center", transform: [{ rotate: row.expanded ? "90deg" : "0deg" }] }}
                    >
                      <RiArrowRightSLine width={DISCLOSURE} height={DISCLOSURE} fill={theme.colors.textSecondary} />
                    </Pressable>
                  ) : (
                    <View style={{ width: DISCLOSURE }} />
                  )
                ) : null}
                {slots ? (
                  row.kind === "app" ? (
                    <AppIcon path={row.icon} color={theme.colors.textSecondary} />
                  ) : (
                    <View style={{ width: ICON }} />
                  )
                ) : null}
                <Text variant="body-2-medium" numberOfLines={1} style={{ flexShrink: 1, color: theme.colors.text }}>
                  {name}
                </Text>
              </View>
            </TableCell>
          ) : (
            <TableCell key={index}>{text}</TableCell>
          ),
        )}
      </TableRow>
    </Pressable>
  );
});

const keyOf = (row: RowView) => row.key;
const layoutOf = (_: ArrayLike<RowView> | null | undefined, index: number) => ({
  length: ROW_HEIGHT,
  offset: ROW_HEIGHT * index,
  index,
});

export function ActivityMonitor({ source, locale }: ActivityMonitorProps) {
  const words = messagesFor(locale);
  const theme = useTheme();
  const minWidth = React.useMemo(() => Math.max(820, narrowest(words)), [words]);
  useWindowSizeLimits({ minWidth, minHeight: 480 });

  const [view, setView] = React.useState<ActivityView>("cpu");
  const [mine, setMine] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const [sorts, setSorts] = React.useState<Record<ActivityView, Sort>>(DEFAULT_SORTS);
  const [expanded, setExpanded] = React.useState<ReadonlySet<string>>(() => new Set());
  const [selectedKey, setSelectedKey] = React.useState<string | null>(null);
  const [sample, setSample] = React.useState<ActivitySample | null>(null);
  const [notice, setNotice] = React.useState<{ outcome: Exclude<QuitOutcome, null>; samples: number } | null>(null);
  const [quitting, setQuitting] = React.useState<Row | null>(null);
  const [inspecting, setInspecting] = React.useState<InspectTarget | null>(null);
  const historyRef = React.useRef<History | null>(null);
  if (historyRef.current === null) historyRef.current = new History();
  const history = historyRef.current;

  // The samples, followed from mount to unmount; the view's changes are told as they happen.
  const viewRef = React.useRef(view);
  viewRef.current = view;
  React.useEffect(() => {
    if (!source) return undefined;
    return source.follow(viewRef.current, (next) => {
      history.push(pointOf(next, Date.now()));
      setSample(next);
      // A quit's failure stays a few samples, then goes: the samples are the app's only clock.
      setNotice((shown) => (shown === null ? null : shown.samples > 1 ? { ...shown, samples: shown.samples - 1 } : null));
    });
  }, [source, history]);
  React.useEffect(() => {
    source?.setView(view);
  }, [source, view]);

  // The rows: grouped, sorted, filtered and flattened, each step only when what it reads changes.
  const sort = sorts[view];
  const grouped = React.useMemo(() => (sample ? groupRows(sample, view) : []), [sample, view]);
  const sorted = React.useMemo(() => sortRows(grouped, sort), [grouped, sort]);
  const filtered = React.useMemo(() => filterRows(sorted, { query, mine }), [sorted, query, mine]);
  const flat = React.useMemo(() => {
    const open = (key: string) => expanded.has(key) || filtered.searched.has(key);
    return flattenRows(filtered.rows, open);
  }, [filtered, expanded]);
  const byKey = React.useMemo(() => new Map(flat.map((row) => [row.key, row])), [flat]);
  const viewsRef = React.useRef<ReadonlyMap<string, RowView>>(new Map());
  const rowViews = React.useMemo(() => {
    const next = keepUnchanged(
      viewsRef.current,
      flat.map((row) => rowView(row, view, locale, words.none, expanded.has(row.key) || filtered.searched.has(row.key))),
    );
    viewsRef.current = new Map(next.map((item) => [item.key, item]));
    return next;
  }, [flat, view, locale, words, expanded, filtered]);

  const selected = selectedKey ? (byKey.get(selectedKey) ?? null) : null;
  const quittable = canQuit(selected);
  const inspectable = canInspect(selected);
  const quitHint = selected && !quittable && selected.kind !== "interface" && !selected.own ? words.needsAdministrator : undefined;

  // The commands, through refs so the rows and the menus keep one function each.
  const state = React.useRef({ selected, rowViews, quittable, inspectable });
  state.current = { selected, rowViews, quittable, inspectable };

  const inspect = React.useCallback((row: Row | null) => {
    if (!row || !canInspect(row)) return;
    if (row.kind === "process") setInspecting({ pid: row.pid, started: row.started, name: row.name });
    else if (row.kind === "app" && row.pid !== null && row.started !== null) {
      setInspecting({ pid: row.pid, started: row.started, name: row.name });
    }
  }, []);
  const askQuit = React.useCallback(() => {
    const { selected: row, quittable: may } = state.current;
    if (row && may) setQuitting(row);
  }, []);
  const inspectSelected = React.useCallback(() => inspect(state.current.selected), [inspect]);

  const quit = React.useCallback(
    async (row: Row, force: boolean) => {
      if (!source || row.kind === "interface") return;
      const answer = row.kind === "app" ? await source.quitApp(row.id, force) : await source.quit(row.pid, row.started, force);
      const outcome = quitOutcome(answer);
      setNotice(outcome ? { outcome, samples: NOTICE_SAMPLES } : null);
    },
    [source],
  );

  const lastPress = React.useRef({ key: "", at: 0 });
  const press = React.useCallback(
    (key: string) => {
      const now = Date.now();
      const double = lastPress.current.key === key && now - lastPress.current.at < DOUBLE_PRESS_MS;
      lastPress.current = { key, at: double ? 0 : now };
      setSelectedKey(key);
      if (double) inspect(byKeyRef.current.get(key) ?? null);
    },
    [inspect],
  );
  const byKeyRef = React.useRef(byKey);
  byKeyRef.current = byKey;
  const toggle = React.useCallback((key: string) => {
    setExpanded((open) => {
      const next = new Set(open);
      if (!next.delete(key)) next.add(key);
      return next;
    });
  }, []);

  // The table's scroll, kept to bring a row the keys select into view.
  const listRef = React.useRef<FlatList<RowView>>(null);
  const scroll = React.useRef({ y: 0, height: 0 });
  const onScroll = React.useCallback((event: NativeSyntheticEvent<NativeScrollEvent>) => {
    scroll.current.y = event.nativeEvent.contentOffset.y;
  }, []);
  const onListLayout = React.useCallback((event: LayoutChangeEvent) => {
    scroll.current.height = event.nativeEvent.layout.height;
  }, []);
  const reveal = (index: number) => {
    const top = index * ROW_HEIGHT;
    const { y, height } = scroll.current;
    if (top < y) listRef.current?.scrollToOffset({ offset: top, animated: false });
    else if (top + ROW_HEIGHT > y + height) listRef.current?.scrollToOffset({ offset: top + ROW_HEIGHT - height, animated: false });
  };

  // ↑ ↓ move the selection, → ← open and close an app (← from a process goes to its app), Return
  // inspects, as macOS's outline. ⌘I and ⌥⌘Q are the View menu's.
  useWindowKeys(
    (event) => {
      if (event.altKey || event.ctrlKey || event.metaKey) return false;
      const rows = state.current.rowViews;
      const index = rows.findIndex((row) => row.key === selectedKey);
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        if (rows.length === 0) return true;
        const next =
          index < 0
            ? event.key === "ArrowDown"
              ? 0
              : rows.length - 1
            : Math.max(0, Math.min(rows.length - 1, index + (event.key === "ArrowDown" ? 1 : -1)));
        setSelectedKey(rows[next].key);
        reveal(next);
        return true;
      }
      const row = state.current.selected;
      if (event.key === "ArrowRight" || event.key === "ArrowLeft") {
        if (!row) return false;
        const opening = event.key === "ArrowRight";
        if (row.kind === "app" && row.children.length > 0) {
          const open = expanded.has(row.key) || filtered.searched.has(row.key);
          if (open !== opening) toggle(row.key);
          return true;
        }
        if (!opening && row.kind === "process" && row.depth === 1) {
          for (let i = index - 1; i >= 0; i -= 1) {
            if (rows[i].kind === "app") {
              setSelectedKey(rows[i].key);
              reveal(i);
              break;
            }
          }
          return true;
        }
        return false;
      }
      if (event.key === "Enter") {
        inspect(row);
        return true;
      }
      return false;
    },
    { enabled: quitting === null && inspecting === null },
  );

  const changeView = React.useCallback((next: ActivityView) => {
    setView(next);
    setSelectedKey(null);
  }, []);

  const renderItem = React.useCallback(
    ({ item }: ListRenderItemInfo<RowView>) => (
      <ActivityRow
        row={item}
        selected={item.key === selectedKey}
        slots={view !== "network"}
        onPress={press}
        onToggle={toggle}
        words={words}
      />
    ),
    [selectedKey, view, press, toggle, words],
  );

  const columns = COLUMNS[view];
  const empty = !source
    ? words.unavailable
    : sample === null
      ? ""
      : query.trim()
        ? words.noMatches
        : view === "network"
          ? words.noInterfaces
          : words.noProcesses;
  const quitName = quitting?.name ?? "";
  const noticeText = notice ? words.quit[notice.outcome] : null;

  const toolbar = (
    <PageHeader
      presentation="bar"
      border="always"
      onBack={null}
      title={words.title}
      subtitle={mine ? words.myProcesses : words.allProcesses}
      actions={
        <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
          <Button
            icon={RiCloseCircleLine}
            appearance="plain"
            tone="neutral"
            size="sm"
            accessibilityLabel={words.quitProcess}
            accessibilityHint={quitHint}
            title={quitHint ?? words.quitProcess}
            disabled={!quittable}
            onPress={askQuit}
            testID="activity-quit"
          />
          <Button
            icon={RiInformationLine}
            appearance="plain"
            tone="neutral"
            size="sm"
            accessibilityLabel={words.inspect}
            title={words.inspect}
            disabled={!inspectable}
            onPress={inspectSelected}
            testID="activity-inspect"
          />
          <SegmentedControl<ActivityView>
            type="tabs"
            size="sm"
            label={words.viewsLabel}
            value={view}
            onValueChange={changeView}
            testID="activity-views"
          >
            {VIEWS.map((item) => (
              <SegmentedControlItem key={item} value={item} testID={`activity-view-${item}`}>
                <SegmentedControlItemText>{words.views[item]}</SegmentedControlItemText>
              </SegmentedControlItem>
            ))}
          </SegmentedControl>
          <Search
            value={query}
            onChangeText={setQuery}
            onClearText={() => setQuery("")}
            placeholder={words.search}
            accessibilityLabel={words.search}
            testID="activity-search"
            containerStyle={{ width: 180 }}
          />
        </View>
      }
    />
  );

  return (
    <Screen header={toolbar} scroll="none" testID="activity-monitor">
      {/* The View menu: in the session's menu bar on OxyOS, in the window elsewhere (ADR 0021). */}
      <Menubar>
        <MenubarMenu value="view">
          <MenubarTrigger>{words.view}</MenubarTrigger>
          <MenubarContent>
            <MenubarCheckboxItem checked={!mine} onCheckedChange={() => setMine(false)}>
              {words.allProcesses}
            </MenubarCheckboxItem>
            <MenubarCheckboxItem checked={mine} onCheckedChange={() => setMine(true)}>
              {words.myProcesses}
            </MenubarCheckboxItem>
            <MenubarSeparator />
            <MenubarItem shortcut="Mod+Alt+Q" disabled={!quittable} onPress={askQuit}>
              {words.quitProcess}
            </MenubarItem>
            <MenubarItem shortcut="Mod+I" disabled={!inspectable} onPress={inspectSelected}>
              {words.inspectProcess}
            </MenubarItem>
          </MenubarContent>
        </MenubarMenu>
      </Menubar>
      <View style={{ flex: 1 }}>
        <Table
          size="sm"
          accessibilityLabel={view === "network" ? words.interfacesLabel : words.tableLabel}
          testID="activity-table"
          style={{ flex: 1 }}
          containerStyle={{ flex: 1 }}
        >
          <TableHeader>
            {columns.map((key) => (
              <TableColumn
                key={key}
                {...columnLayout(key, columnLabel(key, view, words))}
                onSort={() => setSorts((all) => ({ ...all, [view]: nextSort(all[view], key) }))}
                sortDirection={sort.key === key ? sort.direction : "none"}
                testID={`activity-column-${key}`}
              >
                {columnLabel(key, view, words)}
              </TableColumn>
            ))}
          </TableHeader>
          <FlatList
            ref={listRef}
            style={{ flex: 1 }}
            data={rowViews}
            keyExtractor={keyOf}
            renderItem={renderItem}
            extraData={selectedKey}
            getItemLayout={layoutOf}
            initialNumToRender={24}
            maxToRenderPerBatch={16}
            windowSize={5}
            onScroll={onScroll}
            scrollEventThrottle={32}
            onLayout={onListLayout}
            ListEmptyComponent={
              empty ? (
                <View style={{ height: 160, alignItems: "center", justifyContent: "center", paddingHorizontal: 24 }}>
                  <Text variant="body-regular" style={{ color: theme.colors.textSecondary, textAlign: "center" }} testID="activity-empty">
                    {empty}
                  </Text>
                </View>
              ) : undefined
            }
          />
        </Table>
        {noticeText || view === "network" ? (
          <View style={{ paddingHorizontal: 20, paddingVertical: 6, borderTopWidth: 1, borderTopColor: theme.colors.borderLight }}>
            <Text
              variant="caption-1-regular"
              accessibilityLiveRegion={noticeText ? "polite" : undefined}
              testID={noticeText ? "activity-notice" : "activity-network-note"}
              style={{ color: noticeText ? theme.colors.error : theme.colors.textSecondary }}
            >
              {noticeText ?? words.perAppNetwork}
            </Text>
          </View>
        ) : null}
        <Summary view={view} sample={sample} history={history} version={history.version} words={words} locale={locale} />
      </View>
      <Dialog
        open={quitting !== null}
        onClose={() => setQuitting(null)}
        title={quitting?.kind === "app" ? words.quit.appTitle(quitName) : words.quit.processTitle}
        description={words.quit.description(quitName)}
        actions={[
          {
            label: words.quit.forceQuit,
            color: "cancel",
            onPress: () => quitting && quit(quitting, true),
            testID: "activity-quit-force",
          },
          { label: words.quit.cancel, color: "cancel", testID: "activity-quit-cancel" },
          {
            label: words.quit.quit,
            color: "default",
            initialFocus: true,
            onPress: () => quitting && quit(quitting, false),
            testID: "activity-quit-confirm",
          },
        ]}
        testID="activity-quit-dialog"
      />
      <Inspector target={inspecting} source={source} words={words} locale={locale} onClose={() => setInspecting(null)} />
    </Screen>
  );
}

