/**
 * The panel under Activity Monitor's table, as macOS's: each view's figures for the whole machine
 * beside a graph of its last three minutes. Each graph is one SVG path per series, drawn again only
 * when a sample adds a point (the history's `version`) or the panel changes size.
 */
import * as React from "react";
import { View, type LayoutChangeEvent } from "react-native";
import Svg, { Line, Path } from "react-native-svg";
import { useTheme } from "@oxy.so/bloom/theme";
import { Text } from "@oxy.so/bloom/typography";
import type { Messages } from "./messages";
import {
  areaPath,
  formatBytes,
  formatCount,
  formatImpact,
  formatLoad,
  formatPerSecond,
  formatRate,
  formatShare,
  peakOf,
  segmentsOf,
  stacked,
  type ActivitySample,
  type ActivityView,
  type History,
  type HistoryPoint,
  type MemoryLevel,
} from "./model";

/** The panel's height, as macOS's. */
export const SUMMARY_HEIGHT = 150;

/** Apple's system colours, which read on a light and a dark graph alike. */
const RED = "#ff453a";
const BLUE = "#0a84ff";
const GREEN = "#30d158";
const YELLOW = "#ffd60a";
const LEVEL_COLORS: Record<MemoryLevel, string> = { normal: GREEN, warning: YELLOW, critical: RED };

type Series = { d: string; color: string };
type GraphKind = "cpu" | "pressure" | "energy" | "disk" | "network";

/** A rate graph's top before anything moves: a kilobyte a second, so idle is flat, not noise. */
const RATE_FLOOR = 1000;
/** Pressure drawn at least this high (of 100): a machine at rest shows a band of its level's colour, not nothing. */
const PRESSURE_FLOOR = 6;

/** The paths of a graph of `kind` over the history, in a `width` by `height` box. */
function graphSeries(kind: GraphKind, points: readonly HistoryPoint[], width: number, height: number): Series[] {
  switch (kind) {
    case "cpu": {
      const system = segmentsOf(points, (p) => p.cpuSystem);
      const user = stacked(segmentsOf(points, (p) => (p.cpuSystem === null ? null : p.cpuUser)), system);
      return [
        { d: areaPath(system, width, height, 100), color: RED },
        { d: areaPath(user, width, height, 100, system), color: BLUE },
      ];
    }
    case "pressure":
      // One path per level, each sample in its level's colour, as macOS colours the graph.
      return (["normal", "warning", "critical"] as const).map((level) => ({
        d: areaPath(
          segmentsOf(points, (p) => (p.level === level && p.pressure !== null ? Math.max(PRESSURE_FLOOR, p.pressure) : null)),
          width,
          height,
          100,
        ),
        color: LEVEL_COLORS[level],
      }));
    case "energy": {
      const energy = segmentsOf(points, (p) => p.energy);
      return [{ d: areaPath(energy, width, height, peakOf(energy, 100)), color: BLUE }];
    }
    case "disk":
    case "network": {
      const up = segmentsOf(points, (p) => (kind === "disk" ? p.readRate : p.receivedRate));
      const down = segmentsOf(points, (p) => (kind === "disk" ? p.writeRate : p.sentRate));
      const max = Math.max(peakOf(up, RATE_FLOOR), peakOf(down, RATE_FLOOR));
      const half = height / 2;
      return [
        { d: areaPath(up, width, half, max), color: BLUE },
        { d: areaPath(down, width, half, max, undefined, { downward: true, top: half }), color: RED },
      ];
    }
  }
}

const Graph = React.memo(function Graph({
  kind,
  title,
  history,
  version,
  testID,
}: {
  kind: GraphKind;
  title: string;
  history: History;
  /** The history's version: a new point draws the graph again. */
  version: number;
  testID: string;
}) {
  const theme = useTheme();
  const [size, setSize] = React.useState({ width: 0, height: 0 });
  const onLayout = React.useCallback((event: LayoutChangeEvent) => {
    const { width, height } = event.nativeEvent.layout;
    setSize((was) => (was.width === width && was.height === height ? was : { width, height }));
  }, []);
  // biome-ignore lint/correctness/useExhaustiveDependencies: `version` is what changes the history
  const series = React.useMemo(
    () => (size.width > 0 ? graphSeries(kind, history.points(), size.width, size.height) : []),
    [kind, history, version, size],
  );
  const mirrored = kind === "disk" || kind === "network";
  return (
    <View style={{ flex: 1, minWidth: 160, gap: 4 }}>
      <Text variant="caption-2-semibold" style={{ color: theme.colors.textSecondary, textAlign: "center" }}>
        {title}
      </Text>
      <View
        onLayout={onLayout}
        testID={testID}
        accessibilityLabel={title}
        style={{
          flex: 1,
          borderWidth: 1,
          borderColor: theme.colors.borderLight,
          borderRadius: 6,
          overflow: "hidden",
          backgroundColor: theme.colors.background,
        }}
      >
        {size.width > 0 ? (
          <Svg width={size.width} height={size.height}>
            {[0.25, 0.5, 0.75].map((at) => (
              <Line
                key={at}
                x1={0}
                x2={size.width}
                y1={Math.round(at * size.height) + 0.5}
                y2={Math.round(at * size.height) + 0.5}
                stroke={theme.colors.borderLight}
                strokeWidth={mirrored && at === 0.5 ? 1 : 0.5}
              />
            ))}
            {series.map((line, index) =>
              line.d ? <Path key={index} d={line.d} fill={line.color} fillOpacity={0.85} /> : null,
            )}
          </Svg>
        ) : null}
      </View>
    </View>
  );
});

/** A figure: its label and its value, with an optional swatch of its graph's colour. */
function Figure({ label, value, color }: { label: string; value: string; color?: string }) {
  const theme = useTheme();
  return (
    <View style={{ flexDirection: "row", alignItems: "center", gap: 6, minHeight: 18 }}>
      {color ? <View style={{ width: 8, height: 8, borderRadius: 2, backgroundColor: color }} /> : null}
      <Text variant="caption-1-regular" numberOfLines={1} style={{ flex: 1, color: theme.colors.textSecondary }}>
        {label}
      </Text>
      <Text variant="caption-1-medium" numberOfLines={1} style={{ color: theme.colors.text, fontVariant: ["tabular-nums"] }}>
        {value}
      </Text>
    </View>
  );
}

function Figures({ children }: { children: React.ReactNode }) {
  return <View style={{ width: 210, justifyContent: "center", gap: 2 }}>{children}</View>;
}

export const Summary = React.memo(function Summary({
  view,
  sample,
  history,
  version,
  words,
  locale,
}: {
  view: ActivityView;
  sample: ActivitySample | null;
  history: History;
  version: number;
  words: Messages;
  locale: string;
}) {
  const theme = useTheme();
  const say = words.summary;
  const system = sample?.system ?? null;
  const dash = "";
  let body: React.ReactNode = null;
  if (view === "cpu") {
    body = (
      <>
        <Figures>
          <Figure label={say.system} value={formatShare(system?.cpu?.system, locale) || dash} color={RED} />
          <Figure label={say.user} value={formatShare(system?.cpu?.user, locale) || dash} color={BLUE} />
          <Figure label={say.idle} value={formatShare(system?.cpu?.idle, locale) || dash} />
        </Figures>
        <Graph kind="cpu" title={say.cpuLoad} history={history} version={version} testID="activity-graph-cpu" />
        <Figures>
          <Figure label={say.threads} value={formatCount(system?.threads, locale)} />
          <Figure label={say.processes} value={formatCount(system?.processes, locale)} />
          <Figure label={say.load} value={system ? formatLoad(system.load, locale) : dash} />
        </Figures>
      </>
    );
  } else if (view === "memory") {
    const memory = system?.memory;
    body = (
      <>
        <Graph kind="pressure" title={say.memoryPressure} history={history} version={version} testID="activity-graph-memory" />
        <Figures>
          <Figure label={say.physicalMemory} value={formatBytes(memory?.total, locale)} />
          <Figure label={say.memoryUsed} value={formatBytes(memory?.used, locale)} />
          <Figure label={say.cachedFiles} value={formatBytes(memory?.cached, locale)} />
          <Figure label={say.swapUsed} value={formatBytes(memory?.swapUsed, locale)} />
        </Figures>
        <Figures>
          <Figure label={say.appMemory} value={formatBytes(memory?.app, locale)} />
          <Figure label={say.wired} value={formatBytes(memory?.wired, locale)} />
          <Figure label={say.compressed} value={formatBytes(memory?.compressed, locale)} />
        </Figures>
      </>
    );
  } else if (view === "energy") {
    const points = history.points();
    const last = points.length > 0 ? points[points.length - 1].energy : null;
    const running = sample?.energy?.filter((app) => app.running).length ?? null;
    body = (
      <>
        <Graph kind="energy" title={say.energyImpact} history={history} version={version} testID="activity-graph-energy" />
        <Figures>
          <Figure label={say.totalImpact} value={formatImpact(last, locale)} color={BLUE} />
          <Figure label={say.runningApps} value={formatCount(running, locale)} />
        </Figures>
      </>
    );
  } else if (view === "disk") {
    const disk = system?.disk;
    body = (
      <>
        <Figures>
          <Figure label={say.readsIn} value={formatCount(disk?.reads, locale)} />
          <Figure label={say.writesOut} value={formatCount(disk?.writes, locale)} />
          <Figure label={say.readsInRate} value={formatPerSecond(disk?.readsRate, locale)} />
          <Figure label={say.writesOutRate} value={formatPerSecond(disk?.writesRate, locale)} />
        </Figures>
        <Graph kind="disk" title={say.io} history={history} version={version} testID="activity-graph-disk" />
        <Figures>
          <Figure label={say.dataRead} value={formatBytes(disk?.read, locale)} />
          <Figure label={say.dataWritten} value={formatBytes(disk?.written, locale)} />
          <Figure label={say.dataReadRate} value={formatRate(disk?.readRate, locale)} color={BLUE} />
          <Figure label={say.dataWrittenRate} value={formatRate(disk?.writeRate, locale)} color={RED} />
        </Figures>
      </>
    );
  } else {
    const network = system?.network;
    body = (
      <>
        <Figures>
          <Figure label={say.packetsIn} value={formatCount(network?.packetsIn, locale)} />
          <Figure label={say.packetsOut} value={formatCount(network?.packetsOut, locale)} />
          <Figure label={say.packetsInRate} value={formatPerSecond(network?.packetsInRate, locale)} />
          <Figure label={say.packetsOutRate} value={formatPerSecond(network?.packetsOutRate, locale)} />
        </Figures>
        <Graph kind="network" title={say.network} history={history} version={version} testID="activity-graph-network" />
        <Figures>
          <Figure label={say.dataReceived} value={formatBytes(network?.received, locale)} />
          <Figure label={say.dataSent} value={formatBytes(network?.sent, locale)} />
          <Figure label={say.dataReceivedRate} value={formatRate(network?.receivedRate, locale)} color={BLUE} />
          <Figure label={say.dataSentRate} value={formatRate(network?.sentRate, locale)} color={RED} />
        </Figures>
      </>
    );
  }
  return (
    <View
      testID="activity-summary"
      style={{
        height: SUMMARY_HEIGHT,
        flexDirection: "row",
        gap: 20,
        paddingHorizontal: 20,
        paddingVertical: 12,
        borderTopWidth: 1,
        borderTopColor: theme.colors.borderLight,
        backgroundColor: theme.colors.backgroundSecondary,
      }}
    >
      {body}
    </View>
  );
});
