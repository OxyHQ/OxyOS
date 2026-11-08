/**
 * General › Storage (docs/adr/0062-storage.mdx), as a Mac's: the volume's bar of what occupies it
 * (Bloom's `UsageCard`, the free space and its legend), the recommendations that make space
 * (*Empty Trash Automatically*, *Review Files*, *Remove Unused Apps*), and every application with
 * what it occupies, each one's details a sheet whose actions are what can be done to it. The
 * figures are asked when the page is shown and after its own changes, never on a clock; nothing
 * here measures anything.
 */
import * as React from "react";
import { AccessibilityInfo, View } from "react-native";
import { AlertDialog } from "@oxy.so/bloom/alert-dialog";
import { Button } from "@oxy.so/bloom/button";
import { ChoiceList } from "@oxy.so/bloom/choice-list";
import { Dialog, type DialogAction } from "@oxy.so/bloom/dialog";
import { GroupedSection, GroupedSectionItem } from "@oxy.so/bloom/grouped-section";
import { Loading } from "@oxy.so/bloom/loading";
import { Switch } from "@oxy.so/bloom/switch";
import { useTheme } from "@oxy.so/bloom/theme";
import { Text } from "@oxy.so/bloom/typography";
import { UsageCard } from "@oxy.so/bloom/usage-card";
import { messagesLanguage, type Messages } from "./messages";
import type { SettingsActions, SettingsSnapshot } from "./model";
import {
  SETTINGS_ID,
  appTotal,
  storageBar,
  unusedApps,
  type IndexedStorage,
  type LargeFile,
  type StorageActions,
  type StorageApp,
  type StorageCategory,
  type StorageMeasured,
} from "./storage";
import { formatSize } from "./updates";

type Props = { snapshot: SettingsSnapshot; actions: SettingsActions; strings: Messages; locale: string };
type Words = Messages["storage"];

/** How many of the largest files Review Files lists. */
const LARGEST = 50;

const PAGE = { gap: 24 } as const;
const OPTION = { alignItems: "stretch", borderRadius: 10, paddingHorizontal: 4 } as const;

/** The index's sums as one text, to tell whether they changed. */
function sumsOf(told: IndexedStorage | null): string {
  return JSON.stringify(told?.kinds ?? null);
}

/** A day as the page writes it: "8 Nov 2026". */
function dateOf(ms: number, locale: string): string {
  return new Date(ms).toLocaleDateString(locale, { year: "numeric", month: "short", day: "numeric" });
}

export function StoragePage({ snapshot, actions, strings, locale }: Props) {
  const words = strings.storage;
  const language = messagesLanguage(locale);
  const theme = useTheme();
  const storage = actions.storage;
  const bytes = React.useCallback((n: number) => formatSize(n, language), [language]);
  // undefined while measured; null when it could not be.
  const [measured, setMeasured] = React.useState<StorageMeasured | null | undefined>(undefined);
  const [indexed, setIndexed] = React.useState<IndexedStorage | null>(null);
  const [confirmTrash, setConfirmTrash] = React.useState(false);
  const [sheet, setSheet] = React.useState<"files" | "unused" | null>(null);
  const [shownApp, setShownApp] = React.useState<string | null>(null);
  const [failure, setFailure] = React.useState<string | null>(null);
  // After one of the page's own changes to the files (one moved to the Trash), until the index has
  // followed it: what its sums were then.
  const [awaitingIndex, setAwaitingIndex] = React.useState<string | null>(null);
  const awaitingRef = React.useRef<string | null>(null);
  awaitingRef.current = awaitingIndex;
  const asked = React.useRef(0);

  // Measured as the page is shown and after each of its own changes; a measurement asked again
  // replaces the one before.
  const measure = React.useCallback(async () => {
    if (!storage) return;
    const turn = ++asked.current;
    const [volume, files] = await Promise.allSettled([storage.measure(), storage.indexed()]);
    if (turn !== asked.current) return;
    if (volume.status === "rejected") console.warn("[settings] Storage could not be measured:", volume.reason);
    setMeasured(volume.status === "fulfilled" ? volume.value : null);
    const told = files.status === "fulfilled" ? files.value : null;
    setIndexed(told);
    if (awaitingRef.current !== null && sumsOf(told) !== awaitingRef.current) setAwaitingIndex(null);
  }, [storage]);
  React.useEffect(() => {
    void measure();
    return () => {
      asked.current += 1;
    };
  }, [measure]);
  // While the index is still being built, its sums are asked again each time it changes, so
  // Documents and Photos grow as it goes; after one of the page's own changes, once more when the
  // index has followed it. Otherwise nothing is followed.
  const following = indexed?.indexing === true || awaitingIndex !== null;
  React.useEffect(() => {
    if (!following || !storage?.watchIndex) return undefined;
    let live = true;
    const ask = () =>
      storage.indexed().then(
        (told) => {
          if (!live) return;
          setIndexed(told);
          // The index followed the change once its sums are no longer what they were.
          if (awaitingRef.current !== null && sumsOf(told) !== awaitingRef.current) setAwaitingIndex(null);
        },
        (error: unknown) => console.warn("[settings] the index's sums could not be read:", error),
      );
    const unwatch = storage.watchIndex(ask);
    // Asked once more now that it is followed: what changed between the last answer and the
    // subscription is not missed.
    void ask();
    return () => {
      live = false;
      unwatch();
    };
  }, [following, storage]);

  if (!storage) {
    return (
      <GroupedSection>
        <GroupedSectionItem title={words.unavailable} testID="storage-unavailable" />
      </GroupedSection>
    );
  }
  if (measured === undefined) {
    return (
      <View style={PAGE} testID="storage-calculating">
        <Loading />
        <Text style={{ fontSize: 13, textAlign: "center", color: theme.colors.textSecondary }}>{words.calculating}</Text>
      </View>
    );
  }
  if (measured === null) {
    return (
      <GroupedSection>
        <GroupedSectionItem title={words.failed} testID="storage-failed" />
      </GroupedSection>
    );
  }

  const bar = storageBar(measured, indexed);
  // The system and what no category names drawn in the neutral greys, as a Mac draws them; the
  // others in Bloom's chart tones.
  const tone: Partial<Record<StorageCategory, string>> = {
    system: theme.colors.textTertiary,
    other: theme.colors.border,
  };
  const unused = unusedApps(measured.apps, Date.now());
  const unusedBytes = unused.reduce((sum, app) => sum + appTotal(app), 0);
  const app = measured.apps.find((one) => one.id === shownApp) ?? null;
  const emptyAutomatically = snapshot.preferences.emptyTrashAutomatically;

  // Reset… (docs/adr/0061): the session asks the person in the system's sheet; once done, its own
  // data is measured again.
  const reset = async (target: StorageApp) => {
    if (!storage.reset) return;
    const outcome = await storage.reset(target.id);
    if (outcome === "reset") {
      AccessibilityInfo.announceForAccessibility(words.resetDone(target.name));
      setAwaitingIndex(true);
      void measure();
    } else if (outcome === "failed") {
      setFailure(words.resetFailed(target.name));
    }
  };

  const remove = async (target: StorageApp) => {
    const outcome = await storage.remove(target.id);
    if (outcome === "removed") {
      setShownApp(null);
      AccessibilityInfo.announceForAccessibility(words.removed(target.name));
      // Its own data is no file the index holds: only the session's figures change.
      void measure();
    } else if (outcome === "failed") {
      setFailure(words.removeFailed(target.name));
    }
  };

  return (
    <View style={PAGE}>
      <UsageCard
        title={measured.name ?? words.volume}
        usage={{
          max: bar.capacity,
          segments: bar.segments.map(({ key, bytes: value }) => ({
            label: words.categories[key],
            value,
            ...(tone[key] ? { color: tone[key] } : {}),
          })),
        }}
        formatValue={bytes}
        defaultExpanded
        labels={{ freeSpace: words.available }}
        testID="storage-volume"
      />
      {indexed?.indexing ? (
        <Text style={{ fontSize: 11, color: theme.colors.textSecondary }} testID="storage-indexing">
          {words.indexing}
        </Text>
      ) : null}
      <GroupedSection title={words.recommendations}>
        <GroupedSectionItem
          title={words.emptyTrash}
          description={words.emptyTrashDescription}
          descriptionLines={0}
          testID="storage-empty-trash"
          rightElement={
            <Switch
              checked={emptyAutomatically}
              // Turned on, as a Mac asks before it erases anything; turned off at once.
              onCheckedChange={(on: boolean) =>
                on ? setConfirmTrash(true) : actions.setPreference("emptyTrashAutomatically", false)
              }
              accessibilityLabel={words.emptyTrash}
              testID="storage-empty-trash-switch"
            />
          }
        />
        <GroupedSectionItem
          title={words.reviewFiles}
          description={words.reviewFilesDescription}
          descriptionLines={0}
          testID="storage-review"
          rightElement={
            <Button size="sm" appearance="outline" tone="neutral" onPress={() => setSheet("files")} testID="storage-review-files">
              {words.reviewFilesButton}
            </Button>
          }
        />
        {unused.length > 0 ? (
          <GroupedSectionItem
            title={words.unusedApps}
            description={words.unusedAppsDescription(unused.length, bytes(unusedBytes))}
            descriptionLines={0}
            testID="storage-unused"
            rightElement={
              <Button size="sm" appearance="outline" tone="neutral" onPress={() => setSheet("unused")} testID="storage-review-apps">
                {words.reviewApps}
              </Button>
            }
          />
        ) : null}
      </GroupedSection>
      <GroupedSection title={words.applications}>
        {measured.apps.length === 0 ? (
          <GroupedSectionItem title={words.noApplications} testID="storage-apps-none" />
        ) : (
          measured.apps.map((one) => (
            <GroupedSectionItem
              key={one.id}
              title={one.name}
              description={one.lastUsed !== null ? words.lastOpened(dateOf(one.lastUsed, locale)) : undefined}
              value={bytes(appTotal(one))}
              onPress={() => setShownApp(one.id)}
              accessibilityLabel={`${one.name}, ${bytes(appTotal(one))}`}
              accessibilityHint={words.appDetails(one.name)}
              testID={`storage-app-${one.id}`}
            />
          ))
        )}
      </GroupedSection>
      <AlertDialog
        open={confirmTrash}
        onOpenChange={(open) => {
          if (!open) setConfirmTrash(false);
        }}
        title={words.emptyTrashTitle}
        description={words.emptyTrashConfirm}
        confirmLabel={words.turnOn}
        cancelLabel={words.cancel}
        defaultAction="cancel"
        onConfirm={() => {
          setConfirmTrash(false);
          actions.setPreference("emptyTrashAutomatically", true);
        }}
        testID="storage-empty-trash-confirm"
      />
      <ApplicationSheet
        app={app}
        words={words}
        bytes={bytes}
        locale={locale}
        onClose={() => setShownApp(null)}
        onRemove={(target) => void remove(target)}
        onReset={storage.reset ? (target) => void reset(target) : undefined}
      />
      <LargeFilesSheet
        open={sheet === "files"}
        storage={storage}
        words={words}
        bytes={bytes}
        locale={locale}
        onClose={() => setSheet(null)}
        onChanged={() => {
          setAwaitingIndex(sumsOf(indexed));
          void measure();
        }}
        onFailed={setFailure}
      />
      <UnusedAppsSheet
        open={sheet === "unused"}
        apps={unused}
        words={words}
        bytes={bytes}
        locale={locale}
        onClose={() => setSheet(null)}
        onRemove={(target) => void remove(target)}
      />
      <AlertDialog
        open={failure !== null}
        onOpenChange={(open) => {
          if (!open) setFailure(null);
        }}
        title={failure ?? ""}
        confirmLabel={words.ok}
        hideCancel
        testID="storage-failure"
      />
    </View>
  );
}

/**
 * An application's details, as a Mac's Storage › Applications shows them: its package, its own
 * data and its caches, when it was last opened, and what can be done to it, each an action of the
 * sheet: Reset… (docs/adr/0061-crashes-and-resetting-an-application.mdx: its own data and
 * settings erased, the session asking the person in the system's sheet; never Settings' own) and
 * Delete App… for one the person installed.
 */
function ApplicationSheet({
  app,
  words,
  bytes,
  locale,
  onClose,
  onRemove,
  onReset,
}: {
  app: StorageApp | null;
  words: Words;
  bytes: (n: number) => string;
  locale: string;
  onClose: () => void;
  onRemove: (app: StorageApp) => void;
  /** Absent where the platform cannot reset an application. */
  onReset?: (app: StorageApp) => void;
}) {
  const shown = React.useRef(app);
  if (app) shown.current = app;
  const one = shown.current;
  const actions: DialogAction[] = [
    ...(onReset && one && one.id !== SETTINGS_ID
      ? [
          {
            label: words.resetApp,
            onPress: () => {
              if (one) onReset(one);
            },
            shouldCloseOnPress: false,
            testID: "storage-app-reset",
          },
        ]
      : []),
    ...(one?.removable
      ? [
          {
            label: words.deleteApp,
            color: "destructive" as const,
            onPress: () => {
              if (one) onRemove(one);
            },
            shouldCloseOnPress: false,
            testID: "storage-app-delete",
          },
        ]
      : []),
    { label: words.done, color: "cancel" as const, testID: "storage-app-done" },
  ];
  return (
    <Dialog
      open={app !== null}
      onClose={onClose}
      title={one?.name ?? ""}
      description={one?.lastUsed != null ? words.lastOpened(dateOf(one.lastUsed, locale)) : undefined}
      actions={actions}
      testID="storage-app-sheet"
    >
      {one ? (
        <GroupedSection>
          {one.package !== null ? (
            <GroupedSectionItem title={words.appParts.package} value={bytes(one.package)} testID="storage-app-package" />
          ) : null}
          <GroupedSectionItem title={words.appParts.data} value={bytes(one.data)} testID="storage-app-data" />
          <GroupedSectionItem title={words.appParts.caches} value={bytes(one.caches)} testID="storage-app-caches" />
        </GroupedSection>
      ) : null}
    </Dialog>
  );
}

/**
 * Review Files: the home's largest files, the largest first, from the search index; the chosen one
 * shown in Dropsea, or moved to the Trash once the person confirms (the sheet asks in place).
 */
function LargeFilesSheet({
  open,
  storage,
  words,
  bytes,
  locale,
  onClose,
  onChanged,
  onFailed,
}: {
  open: boolean;
  storage: StorageActions;
  words: Words;
  bytes: (n: number) => string;
  locale: string;
  onClose: () => void;
  onChanged: () => void;
  onFailed: (message: string) => void;
}) {
  const theme = useTheme();
  const [files, setFiles] = React.useState<LargeFile[] | null>(null);
  const [selected, setSelected] = React.useState<string | null>(null);
  const [confirming, setConfirming] = React.useState<LargeFile | null>(null);
  React.useEffect(() => {
    if (!open) {
      setConfirming(null);
      return undefined;
    }
    let live = true;
    setFiles(null);
    storage.largest(LARGEST).then(
      (found) => {
        if (live) setFiles(found);
      },
      (error: unknown) => {
        console.warn("[settings] the largest files could not be read:", error);
        if (live) setFiles([]);
      },
    );
    return () => {
      live = false;
    };
  }, [open, storage]);
  const chosen = files?.find((file) => file.uri === selected) ?? files?.[0] ?? null;
  const reveal = (file: LargeFile) =>
    storage.reveal(file.uri).catch((error: unknown) => {
      console.warn("[settings] not shown in Dropsea:", error);
      onFailed(words.revealFailed(file.name));
    });
  const moveToTrash = async (file: LargeFile) => {
    setConfirming(null);
    try {
      await storage.trash(file.uri);
    } catch (error) {
      console.warn("[settings] not moved to the Trash:", error);
      onFailed(words.moveFailed(file.name));
      return;
    }
    setFiles((now) => {
      const left = (now ?? []).filter((one) => one.uri !== file.uri);
      setSelected(left[0]?.uri ?? null);
      return left;
    });
    AccessibilityInfo.announceForAccessibility(words.moved(file.name));
    onChanged();
  };
  const actions: DialogAction[] = confirming
    ? [
        {
          label: words.moveToTrash,
          color: "destructive",
          onPress: () => void moveToTrash(confirming),
          shouldCloseOnPress: false,
          testID: "storage-files-trash-confirm",
        },
        {
          label: words.cancel,
          color: "cancel",
          onPress: () => setConfirming(null),
          shouldCloseOnPress: false,
          // The safe answer is where the keyboard starts, as a Mac's alert starts on Cancel.
          initialFocus: true,
          testID: "storage-files-trash-cancel",
        },
      ]
    : [{ label: words.done, color: "cancel", testID: "storage-files-done" }];
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={confirming ? words.deleteFileTitle(confirming.name) : words.largeFiles}
      description={confirming ? words.deleteFileDescription : words.largeFilesDescription}
      actions={actions}
      testID="storage-files-sheet"
    >
      {confirming ? null : files === null ? (
        <Loading />
      ) : files.length === 0 ? (
        <Text style={{ fontSize: 13, color: theme.colors.textSecondary }} testID="storage-files-none">
          {words.noLargeFiles}
        </Text>
      ) : (
        <View style={{ gap: 8 }}>
          <ChoiceList<string>
            label={words.largeFiles}
            value={(chosen ?? files[0]).uri}
            onValueChange={setSelected}
            orientation="vertical"
            style={{ gap: 2 }}
            optionStyle={OPTION}
            options={files.map((file) => ({
              value: file.uri,
              label: `${file.name}, ${bytes(file.size)}`,
              testID: `storage-file-${file.name}`,
            }))}
            renderContent={(option, { selected: isSelected }) => {
              const file = files.find((one) => one.uri === option.value);
              return file ? (
                <ItemRow
                  title={file.name}
                  detail={words.fileLine(
                    file.location,
                    bytes(file.size),
                    file.modified !== null ? dateOf(file.modified, locale) : "",
                  )}
                  selected={isSelected}
                />
              ) : null;
            }}
            testID="storage-files"
          />
          <View style={{ flexDirection: "row", gap: 8 }}>
            <Button
              size="sm"
              appearance="outline"
              tone="neutral"
              disabled={!chosen}
              accessibilityLabel={chosen ? words.revealLabel(chosen.name) : words.reveal}
              onPress={() => {
                if (chosen) void reveal(chosen);
              }}
              testID="storage-files-reveal"
            >
              {words.reveal}
            </Button>
            <Button
              size="sm"
              appearance="outline"
              tone="neutral"
              disabled={!chosen}
              accessibilityLabel={chosen ? words.deleteFileLabel(chosen.name) : words.deleteFile}
              onPress={() => {
                if (chosen) setConfirming(chosen);
              }}
              testID="storage-files-delete"
            >
              {words.deleteFile}
            </Button>
          </View>
        </View>
      )}
    </Dialog>
  );
}

/** Remove Unused Apps: the applications not opened in three months, the largest first, the chosen
    one deleted through the store's uninstall, which the person confirms in the system's sheet. */
function UnusedAppsSheet({
  open,
  apps,
  words,
  bytes,
  locale,
  onClose,
  onRemove,
}: {
  open: boolean;
  apps: StorageApp[];
  words: Words;
  bytes: (n: number) => string;
  locale: string;
  onClose: () => void;
  onRemove: (app: StorageApp) => void;
}) {
  const [selected, setSelected] = React.useState<string | null>(null);
  const chosen = apps.find((app) => app.id === selected) ?? apps[0] ?? null;
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={words.unusedTitle}
      description={words.unusedDescription}
      actions={[{ label: words.done, color: "cancel", testID: "storage-unused-done" }]}
      testID="storage-unused-sheet"
    >
      {chosen ? (
        <View style={{ gap: 8 }}>
          <ChoiceList<string>
            label={words.unusedTitle}
            value={chosen.id}
            onValueChange={setSelected}
            orientation="vertical"
            style={{ gap: 2 }}
            optionStyle={OPTION}
            options={apps.map((app) => ({
              value: app.id,
              label: `${app.name}, ${bytes(appTotal(app))}`,
              testID: `storage-unused-${app.id}`,
            }))}
            renderContent={(option, { selected: isSelected }) => {
              const app = apps.find((one) => one.id === option.value);
              return app ? (
                <ItemRow
                  title={app.name}
                  detail={[
                    bytes(appTotal(app)),
                    app.lastUsed !== null ? words.lastOpened(dateOf(app.lastUsed, locale)) : "",
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                  selected={isSelected}
                />
              ) : null;
            }}
            testID="storage-unused-list"
          />
          <View style={{ flexDirection: "row" }}>
            <Button
              size="sm"
              appearance="outline"
              tone="neutral"
              accessibilityLabel={words.deleteAppLabel(chosen.name)}
              onPress={() => onRemove(chosen)}
              testID="storage-unused-delete"
            >
              {words.deleteApp}
            </Button>
          </View>
        </View>
      ) : null}
    </Dialog>
  );
}

/** A row of a sheet's list, drawn: its title and, small, what it is. Decorative: the list's option
    is the control, named by its label. */
function ItemRow({ title, detail, selected }: { title: string; detail: string; selected: boolean }) {
  const theme = useTheme();
  const ink = selected ? theme.colors.primaryForeground : theme.colors.text;
  return (
    <View
      style={{
        paddingVertical: 6,
        paddingHorizontal: 10,
        borderRadius: 10,
        backgroundColor: selected ? theme.colors.primary : "transparent",
      }}
    >
      <Text numberOfLines={1} style={{ color: ink, fontSize: 14 }}>
        {title}
      </Text>
      <Text numberOfLines={1} style={{ color: selected ? ink : theme.colors.textSecondary, fontSize: 11 }}>
        {detail}
      </Text>
    </View>
  );
}
