/**
 * Activity Monitor's inspector (⌘I, or a row's double click), as macOS's: the process's parent,
 * user, start and CPU time, then its memory, its statistics and its open files and ports in tabs.
 * The session reads a process only when asked: once as the sheet opens and again on Refresh, never
 * on a timer. Another user's process says what needs an administrator instead of its details.
 */
import * as React from "react";
import { View } from "react-native";
import { Dialog } from "@oxy.so/bloom/dialog";
import { SegmentedControl, SegmentedControlItem, SegmentedControlItemText } from "@oxy.so/bloom/segmented-control";
import { useTheme } from "@oxy.so/bloom/theme";
import { Text } from "@oxy.so/bloom/typography";
import type { Messages } from "./messages";
import { formatBytes, formatCount, formatCpuTime, inspectionOf, type ActivitySource, type Inspection } from "./model";

/** The process the inspector is open on. */
export type InspectTarget = { pid: number; started: number; name: string };

type Tab = "memory" | "statistics" | "files";

function Fact({ label, value, testID }: { label: string; value: string; testID?: string }) {
  const theme = useTheme();
  return (
    <View style={{ flexDirection: "row", gap: 12, paddingVertical: 3 }}>
      <Text variant="body-2-regular" style={{ width: 150, color: theme.colors.textSecondary, textAlign: "right" }}>
        {label}
      </Text>
      <Text variant="body-2-regular" selectable testID={testID} style={{ flex: 1, color: theme.colors.text }}>
        {value}
      </Text>
    </View>
  );
}

function Note({ children, testID }: { children: string; testID?: string }) {
  const theme = useTheme();
  return (
    <Text variant="body-2-regular" testID={testID} style={{ color: theme.colors.textSecondary, paddingVertical: 8 }}>
      {children}
    </Text>
  );
}

function MemoryTab({ inspection, words, locale }: { inspection: Inspection; words: Messages["inspector"]; locale: string }) {
  const memory = inspection.memory;
  if (!memory) return <Note testID="activity-inspector-restricted">{words.restricted}</Note>;
  return (
    <View>
      <Fact label={words.footprint} value={formatBytes(memory.footprint, locale)} />
      <Fact label={words.unique} value={formatBytes(memory.unique, locale)} />
      <Fact label={words.proportional} value={formatBytes(memory.proportional, locale)} />
      <Fact label={words.resident} value={formatBytes(memory.resident, locale)} />
      <Fact label={words.swap} value={formatBytes(memory.swap, locale)} />
      <Fact label={words.gpu} value={formatBytes(memory.gpu, locale)} />
    </View>
  );
}

function StatisticsTab({ inspection, words, locale }: { inspection: Inspection; words: Messages["inspector"]; locale: string }) {
  const command = inspection.arguments?.join(" ") ?? null;
  return (
    <View>
      <Fact label={words.threads} value={formatCount(inspection.threads, locale)} />
      <Fact label={words.cpuTime} value={formatCpuTime(inspection.cpuTime, locale)} />
      {inspection.app ? <Fact label={words.application} value={inspection.app} /> : null}
      {inspection.cgroup ? <Fact label={words.cgroup} value={inspection.cgroup} /> : null}
      {inspection.path ? <Fact label={words.path} value={inspection.path} /> : null}
      {command ? <Fact label={words.command} value={command} testID="activity-inspector-command" /> : null}
      {inspection.restricted ? <Note testID="activity-inspector-restricted">{words.restricted}</Note> : null}
    </View>
  );
}

function FilesTab({ inspection, words }: { inspection: Inspection; words: Messages["inspector"] }) {
  const theme = useTheme();
  if (inspection.restricted || (!inspection.files && !inspection.ports)) {
    return <Note testID="activity-inspector-restricted">{words.restricted}</Note>;
  }
  const heading = (text: string) => (
    <Text variant="caption-2-semibold" style={{ color: theme.colors.textSecondary, marginTop: 8, marginBottom: 2 }}>
      {text.toUpperCase()}
    </Text>
  );
  const line = (fd: number, text: string, key: string) => (
    <View key={key} style={{ flexDirection: "row", gap: 10, paddingVertical: 2 }}>
      <Text variant="caption-1-regular" style={{ width: 32, textAlign: "right", color: theme.colors.textSecondary, fontVariant: ["tabular-nums"] }}>
        {fd}
      </Text>
      <Text variant="caption-1-regular" selectable style={{ flex: 1, color: theme.colors.text }}>
        {text}
      </Text>
    </View>
  );
  const files = inspection.files ?? [];
  const ports = inspection.ports ?? [];
  return (
    <View testID="activity-inspector-files">
      {heading(words.files)}
      {files.length === 0 ? <Note>{words.noFiles}</Note> : files.map((file) => line(file.fd, file.path, `f${file.fd}`))}
      {heading(words.ports)}
      {ports.length === 0 ? (
        <Note>{words.noPorts}</Note>
      ) : (
        ports.map((port) =>
          line(
            port.fd,
            [port.kind.toUpperCase(), port.local, port.remote ? `→ ${port.remote}` : null, port.state ? `(${port.state})` : null]
              .filter(Boolean)
              .join(" "),
            `p${port.fd}`,
          ),
        )
      )}
    </View>
  );
}

export function Inspector({
  target,
  source,
  words,
  locale,
  onClose,
}: {
  target: InspectTarget | null;
  source: ActivitySource | null;
  words: Messages;
  locale: string;
  onClose: () => void;
}) {
  const say = words.inspector;
  const [answer, setAnswer] = React.useState<Inspection | "notFound" | "failed" | null>(null);
  const [tab, setTab] = React.useState<Tab>("memory");
  // Each ask's turn: an answer to an earlier one (another process, a Refresh since) is dropped.
  const turn = React.useRef(0);

  const read = React.useCallback(() => {
    if (!target || !source) return;
    const mine = ++turn.current;
    source.inspect(target.pid, target.started).then((reply) => {
      if (turn.current === mine) setAnswer(inspectionOf(reply));
    });
  }, [source, target]);

  React.useEffect(() => {
    setAnswer(null);
    setTab("memory");
    read();
  }, [read]);

  const started = React.useMemo(() => {
    if (!answer || typeof answer !== "object") return "";
    return new Intl.DateTimeFormat(locale, { dateStyle: "medium", timeStyle: "medium" }).format(new Date(answer.startedAt * 1000));
  }, [answer, locale]);

  let body: React.ReactNode;
  if (answer === null) body = <Note>{say.loading}</Note>;
  else if (answer === "notFound") body = <Note testID="activity-inspector-gone">{say.notFound}</Note>;
  else if (answer === "failed") body = <Note testID="activity-inspector-gone">{words.quit.failed}</Note>;
  else {
    body = (
      <View style={{ gap: 12 }}>
        <View>
          <Fact
            label={say.parent}
            value={answer.parentName ? `${answer.parentName} (${answer.ppid})` : String(answer.ppid)}
            testID="activity-inspector-parent"
          />
          <Fact label={say.user} value={answer.user} testID="activity-inspector-user" />
          <Fact label={say.started} value={started} />
          <Fact label={say.cpuTime} value={formatCpuTime(answer.cpuTime, locale)} />
        </View>
        <SegmentedControl<Tab> type="tabs" size="sm" label={say.tabs} value={tab} onValueChange={setTab} testID="activity-inspector-tabs">
          <SegmentedControlItem value="memory">
            <SegmentedControlItemText>{say.memoryTab}</SegmentedControlItemText>
          </SegmentedControlItem>
          <SegmentedControlItem value="statistics">
            <SegmentedControlItemText>{say.statisticsTab}</SegmentedControlItemText>
          </SegmentedControlItem>
          <SegmentedControlItem value="files">
            <SegmentedControlItemText>{say.filesTab}</SegmentedControlItemText>
          </SegmentedControlItem>
        </SegmentedControl>
        {tab === "memory" ? (
          <MemoryTab inspection={answer} words={say} locale={locale} />
        ) : tab === "statistics" ? (
          <StatisticsTab inspection={answer} words={say} locale={locale} />
        ) : (
          <FilesTab inspection={answer} words={say} />
        )}
      </View>
    );
  }

  return (
    <Dialog
      open={target !== null}
      onClose={onClose}
      title={target ? say.title(target.name, target.pid) : ""}
      maxWidth={560}
      actions={[
        { label: say.refresh, color: "cancel", shouldCloseOnPress: false, onPress: read, testID: "activity-inspector-refresh" },
        { label: say.done, color: "default", initialFocus: true, testID: "activity-inspector-done" },
      ]}
      testID="activity-inspector"
    >
      {body}
    </Dialog>
  );
}
