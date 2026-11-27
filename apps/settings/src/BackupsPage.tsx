/**
 * Settings › General › Backups (docs/adr/0059-backups.mdx, decision 9), as a Mac's System Settings ›
 * General › Time Machine: the backup disks and shares, each with its latest and oldest backup, the
 * space it takes and what is free, and why its last backup failed; the backup running and how far;
 * *Back Up Now* and *Skip This Backup*; for an administrator, *Add Backup Disk…* (a disk connected,
 * or a network share, then the backup password typed twice), removing a destination and
 * *Options…* (the items excluded, *Back Up Automatically*, the menu bar's item).
 *
 * Everything is the session's: the state as it tells it, unasked, as it changes (nothing here is
 * on a clock, docs/performance.mdx), the disks asked as the sheet opens, each change asked of it
 * and said in words where it failed.
 */
import * as React from "react";
import { View } from "react-native";
import { AlertDialog } from "@oxy.so/bloom/alert-dialog";
import { Button } from "@oxy.so/bloom/button";
import { ChoiceList } from "@oxy.so/bloom/choice-list";
import { Dialog, type DialogAction } from "@oxy.so/bloom/dialog";
import { GroupedSection, GroupedSectionItem } from "@oxy.so/bloom/grouped-section";
import {
  RiAddLine,
  RiFolder3Line,
  RiHardDrive2Line,
  RiHistoryLine,
  RiServerLine,
  RiSubtractLine,
} from "@oxy.so/bloom/icons";
import { Loading } from "@oxy.so/bloom/loading";
import { Meter } from "@oxy.so/bloom/stat-bar";
import { Switch } from "@oxy.so/bloom/switch";
import { TextField, TextFieldHint, TextFieldInput } from "@oxy.so/bloom/text-field";
import { useTheme } from "@oxy.so/bloom/theme";
import { Text } from "@oxy.so/bloom/typography";
import {
  backupBytes,
  backupPasswordProblem,
  backupReasonLabel,
  backupWhen,
  canBackUp,
  destinationSummary,
  diskName,
  fileSystemName,
  pathName,
  progressFraction,
  progressKnown,
  progressLine,
  shareOf,
  withExcludedPath,
  type BackupDestination,
  type BackupDisk,
  type BackupShare,
  type BackupState,
  type ShareFields,
} from "./backups";
import type { Messages } from "./messages";
import type { SettingsActions, SettingsSnapshot } from "./model";
import { folderPath, selectedAfterRemoving } from "./spotlight";
import { ListRow, ListRowDetail, ListRowTitle, useListRowInk } from "./ListRow";

type PageProps = { snapshot: SettingsSnapshot; actions: SettingsActions; strings: Messages; locale: string };
type Words = Messages["backups"];

const PAGE = { gap: 20 } as const;
const OPTION = { alignItems: "stretch", borderRadius: 10, paddingHorizontal: 4 } as const;
/** The reasons that are the share's: the person goes back to its form to put them right. */
const SHARE_REASONS = new Set(["invalidHost", "invalidShare", "invalidUser", "invalidDomain", "shareRefused", "unreachable"]);

/** A dialog's content kept while it closes, so its words do not vanish under its animation. */
function useHeld<T>(): [T | null, boolean, (value: T) => void, () => void] {
  const [held, setHeld] = React.useState<{ value: T | null; open: boolean }>({ value: null, open: false });
  const show = React.useCallback((value: T) => setHeld({ value, open: true }), []);
  const hide = React.useCallback(() => setHeld((current) => ({ ...current, open: false })), []);
  return [held.value, held.open, show, hide];
}

function DestinationIcon({ kind, color }: { kind: BackupDestination["kind"]; color?: string }) {
  return kind === "disk" ? <RiHardDrive2Line size="lg" fill={color} /> : <RiServerLine size="lg" fill={color} />;
}

/** A destination's row: its icon, its name and what Time Machine says under it; for an
    administrator, − to remove it. */
function DestinationRow({
  destination,
  now,
  locale,
  words,
  onRemove,
}: {
  destination: BackupDestination;
  now: number;
  locale: string;
  words: Words;
  onRemove: (() => void) | null;
}) {
  const theme = useTheme();
  const summary = destinationSummary(destination, now, locale, words);
  const lines = [summary.status, summary.latest, summary.oldest, summary.space].filter(
    (line): line is string => line !== null,
  );
  const said = [destination.name, ...lines, summary.failure].filter(Boolean).join(", ");
  return (
    <GroupedSectionItem
      title={destination.name}
      icon={<DestinationIcon kind={destination.kind} color={theme.colors.textSecondary} />}
      testID={`backups-destination-${destination.id}`}
      content={
        <View accessible accessibilityLabel={said} style={{ gap: 2, paddingVertical: 2 }}>
          <Text numberOfLines={1} style={{ fontSize: 15, fontWeight: "600", color: theme.colors.text }}>
            {destination.name}
          </Text>
          {lines.map((line) => (
            <Text key={line} style={{ fontSize: 12, lineHeight: 16, color: theme.colors.textSecondary }}>
              {line}
            </Text>
          ))}
          {summary.failure ? (
            <Text style={{ fontSize: 12, lineHeight: 16, color: theme.colors.error }}>{summary.failure}</Text>
          ) : null}
        </View>
      }
      rightElement={
        onRemove ? (
          <Button
            size="sm"
            appearance="plain"
            tone="neutral"
            iconOnly
            icon={RiSubtractLine}
            accessibilityLabel={words.removeLabel(destination.name)}
            onPress={onRemove}
            testID={`backups-remove-${destination.id}`}
          />
        ) : undefined
      }
    />
  );
}

/** The backup running: where to, the bar and what it is doing in a line. */
function Progress({ state, locale, words }: { state: BackupState; locale: string; words: Words }) {
  const theme = useTheme();
  const running = state.running;
  if (!running) return null;
  const destination = state.destinations.find((one) => one.id === running.destination);
  const name = destination?.name ?? "";
  const line = progressLine(running, locale, words);
  const known = progressKnown(running);
  return (
    <GroupedSection>
      <GroupedSectionItem
        title={name}
        icon={destination ? <DestinationIcon kind={destination.kind} color={theme.colors.textSecondary} /> : undefined}
        testID="backups-running"
        content={
          <View style={{ gap: 8, paddingVertical: 4 }}>
            <Text numberOfLines={1} style={{ fontSize: 15, fontWeight: "600", color: theme.colors.text }}>
              {name}
            </Text>
            <Meter
              role="progressbar"
              accessibilityLabel={words.progressLabel(name)}
              valueText={line}
              value={known ? progressFraction(running) : 0}
              indeterminate={!known}
              testID="backups-progress"
            />
            <Text style={{ fontSize: 12, lineHeight: 16, color: theme.colors.textSecondary }} testID="backups-progress-line">
              {line}
            </Text>
          </View>
        }
      />
    </GroupedSection>
  );
}

/** A row whose control is a switch, named by its title. */
function SwitchRow({
  title,
  description,
  value,
  onChange,
  disabled,
  testID,
}: {
  title: string;
  description?: string;
  value: boolean;
  onChange: (on: boolean) => void;
  disabled?: boolean;
  testID: string;
}) {
  return (
    <GroupedSectionItem
      title={title}
      titleNumberOfLines={2}
      description={description}
      descriptionLines={0}
      testID={`${testID}-row`}
      rightElement={
        <Switch
          size="xs"
          checked={value}
          onCheckedChange={onChange}
          disabled={disabled}
          accessibilityLabel={title}
          testID={testID}
        />
      }
    />
  );
}

/** An excluded item, drawn: its icon, its name and, small, its path. Decorative: the list's option
    is the control, named by its label. */
function ExcludedRow({ path, selected }: { path: string; selected: boolean }) {
  const { ink, detail } = useListRowInk(selected);
  return (
    <ListRow selected={selected}>
      <RiFolder3Line size="sm" fill={ink} />
      <View style={{ flex: 1 }}>
        <ListRowTitle color={ink}>{pathName(path)}</ListRowTitle>
        <ListRowDetail color={detail}>{path}</ListRowDetail>
      </View>
    </ListRow>
  );
}

/**
 * Options…: the items excluded from backups (a folder added with + from the system's folder chooser,
 * the selected one removed with −), *Back Up Automatically* and *Show Backups in menu bar*. Each
 * change is the session's: the list and the switch show what it tells once it kept them.
 */
function OptionsSheet({
  open,
  onClose,
  state,
  snapshot,
  actions,
  words,
}: {
  open: boolean;
  onClose: () => void;
  state: BackupState;
  snapshot: SettingsSnapshot;
  actions: SettingsActions;
  words: Words;
}) {
  const theme = useTheme();
  const act = actions.backup;
  const excluded = state.excluded;
  const [selected, setSelected] = React.useState<string | null>(null);
  const [sending, setSending] = React.useState(false);
  const [automatic, setAutomatic] = React.useState<boolean | null>(null);
  const [failure, setFailure] = React.useState<string | null>(null);
  // Shown as asked until the session tells it so.
  React.useEffect(() => {
    if (automatic !== null && state.automatic === automatic) setAutomatic(null);
  }, [automatic, state.automatic]);
  React.useEffect(() => {
    if (open) setFailure(null);
  }, [open]);
  const chosen = selected !== null && excluded.includes(selected) ? selected : (excluded[0] ?? null);
  const send = async (paths: string[]) => {
    if (!act) return;
    setSending(true);
    setFailure(null);
    const failed = await act.setExcluded(paths);
    setSending(false);
    if (failed) setFailure(backupReasonLabel(failed, words));
  };
  const add = async () => {
    const uri = await actions.chooseFolder?.();
    if (!uri) return;
    const path = folderPath(uri);
    const next = withExcludedPath(excluded, path);
    setSelected(path);
    if (next.length !== excluded.length) await send(next);
  };
  const remove = async () => {
    if (chosen === null) return;
    setSelected(selectedAfterRemoving(excluded, chosen));
    await send(excluded.filter((one) => one !== chosen));
  };
  const turnAutomatic = async (on: boolean) => {
    if (!act) return;
    setFailure(null);
    setAutomatic(on);
    const failed = await act.setAutomatic(on);
    if (failed) {
      setAutomatic(null);
      setFailure(backupReasonLabel(failed, words));
    }
  };
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={words.optionsTitle}
      label={words.optionsTitle}
      // Done stays in view below the options, which scroll, as a Mac's sheet keeps its button.
      footer={
        <View style={{ paddingHorizontal: 20, paddingVertical: 12, alignItems: "flex-end" }}>
          <Button onPress={onClose} accessibilityLabel={words.done} testID="backups-options-done">
            {words.done}
          </Button>
        </View>
      }
      testID="backups-options-sheet"
    >
      <View style={{ gap: 16 }}>
        <View style={{ gap: 8 }}>
          <Text style={{ fontSize: 13, fontWeight: "600" }}>{words.excludeTitle}</Text>
          {excluded.length > 0 && chosen !== null ? (
            <ChoiceList<string>
              label={words.excludedList}
              value={chosen}
              onValueChange={setSelected}
              orientation="vertical"
              style={{ gap: 2 }}
              optionStyle={OPTION}
              options={excluded.map((path) => ({
                value: path,
                label: `${pathName(path)}, ${path}`,
                testID: `backups-excluded-${pathName(path)}`,
              }))}
              renderContent={(option, { selected: isSelected }) => (
                <ExcludedRow path={option.value} selected={isSelected} />
              )}
              testID="backups-excluded"
            />
          ) : (
            <Text style={{ fontSize: 13, color: theme.colors.textSecondary }} testID="backups-excluded-none">
              {words.noExclusions}
            </Text>
          )}
          <View style={{ flexDirection: "row", gap: 4 }}>
            {actions.chooseFolder ? (
              <Button
                size="sm"
                appearance="plain"
                tone="neutral"
                iconOnly
                icon={RiAddLine}
                accessibilityLabel={words.addExcluded}
                disabled={sending || !act}
                onPress={() => void add()}
                testID="backups-excluded-add"
              />
            ) : null}
            <Button
              size="sm"
              appearance="plain"
              tone="neutral"
              iconOnly
              icon={RiSubtractLine}
              accessibilityLabel={chosen !== null ? words.removeExcludedNamed(pathName(chosen)) : words.removeExcluded}
              disabled={chosen === null || sending || !act}
              onPress={() => void remove()}
              testID="backups-excluded-remove"
            />
          </View>
          {actions.chooseFolder ? null : (
            <Text style={{ fontSize: 11, color: theme.colors.textSecondary }}>{words.noChooser}</Text>
          )}
        </View>
        <GroupedSection>
          <SwitchRow
            title={words.automatic}
            description={words.automaticDescription}
            value={automatic ?? state.automatic}
            onChange={(on) => void turnAutomatic(on)}
            disabled={!act || automatic !== null}
            testID="backups-automatic"
          />
          <SwitchRow
            title={words.inMenuBar}
            value={snapshot.preferences.backupInMenuBar}
            onChange={(on) => actions.setPreference("backupInMenuBar", on)}
            testID="backups-in-menu-bar"
          />
        </GroupedSection>
        {failure ? (
          <Text accessibilityRole="alert" style={{ fontSize: 13, color: theme.colors.error }} testID="backups-options-failure">
            {failure}
          </Text>
        ) : null}
      </View>
    </Dialog>
  );
}

/** Where *Add Backup Disk…* is: the disks, the share's form, or the backup password for one. */
type Target = { kind: "disk"; disk: BackupDisk; name: string } | { kind: "share"; share: BackupShare; name: string };
type Step = { kind: "choose" } | { kind: "share" } | { kind: "password"; target: Target };

const NO_SHARE: ShareFields = { host: "", share: "", user: "", domain: "" };

/**
 * *Add Backup Disk…*: the disks connected that can hold backups (asked of the session as the sheet
 * opens), or *Network Share…* and its form; then the backup password, typed twice. *Set Up Disk*
 * and *Set Up Share* ask the session, which sets up an encrypted repository (seconds); a failure is
 * said here, and once it is done the sheet closes: the state the session tells shows the new
 * destination and its first backup.
 */
function AddSheet({
  open,
  onClose,
  actions,
  words,
  locale,
}: {
  open: boolean;
  onClose: () => void;
  actions: SettingsActions;
  words: Words;
  locale: string;
}) {
  const theme = useTheme();
  const act = actions.backup;
  const [step, setStep] = React.useState<Step>({ kind: "choose" });
  // undefined while asked; null when the session could not say.
  const [disks, setDisks] = React.useState<BackupDisk[] | null | undefined>(undefined);
  const [fields, setFields] = React.useState<ShareFields>(NO_SHARE);
  const [sharePassword, setSharePassword] = React.useState("");
  const [password, setPassword] = React.useState("");
  const [again, setAgain] = React.useState("");
  const [working, setWorking] = React.useState(false);
  const [failure, setFailure] = React.useState<string | null>(null);
  // The session's requests as they are now: a new actions object does not start the sheet again.
  const latest = React.useRef(act);
  latest.current = act;
  // Each opening starts again, from the disks connected now; passwords never outlive the sheet.
  React.useEffect(() => {
    if (!open) {
      setSharePassword("");
      setPassword("");
      setAgain("");
      return undefined;
    }
    setStep({ kind: "choose" });
    setFields(NO_SHARE);
    setFailure(null);
    setDisks(undefined);
    const asked = latest.current;
    if (!asked) {
      setDisks(null);
      return undefined;
    }
    let live = true;
    asked.disks().then(
      (told) => {
        if (live) setDisks(told);
      },
      () => {
        if (live) setDisks(null);
      },
    );
    return () => {
      live = false;
    };
  }, [open]);

  const close = () => {
    if (!working) onClose();
  };
  const share = shareOf(fields);
  const problem = backupPasswordProblem(password, again);
  const toPassword = (target: Target) => {
    setFailure(null);
    setPassword("");
    setAgain("");
    setStep({ kind: "password", target });
  };
  const setUp = async () => {
    if (!act || working || step.kind !== "password" || problem !== null) return;
    const target = step.target;
    setWorking(true);
    setFailure(null);
    const failed =
      target.kind === "disk"
        ? await act.addDisk(target.disk.uuid, password)
        : await act.addShare(target.share, sharePassword, password);
    setWorking(false);
    if (!failed) {
      setPassword("");
      setAgain("");
      onClose();
      return;
    }
    setFailure(backupReasonLabel(failed, words));
    // What is the share's is put right in its form.
    if (target.kind === "share" && SHARE_REASONS.has(failed)) setStep({ kind: "share" });
  };
  const field = (key: keyof ShareFields) => (text: string) => {
    setFields((now) => ({ ...now, [key]: text }));
    setFailure(null);
  };
  const failureText = failure ? (
    <Text accessibilityRole="alert" style={{ fontSize: 13, color: theme.colors.error }} testID="backups-add-failure">
      {failure}
    </Text>
  ) : null;
  const cancel: DialogAction = { label: words.cancel, color: "cancel", onPress: close, disabled: working, testID: "backups-add-cancel" };
  const back = (to: Step): DialogAction => ({
    label: words.back,
    color: "cancel",
    shouldCloseOnPress: false,
    disabled: working,
    onPress: () => {
      setFailure(null);
      setStep(to);
    },
    testID: "backups-add-back",
  });

  let title: string;
  let description: string | undefined;
  let dialogActions: DialogAction[];
  let content: React.ReactNode;
  if (step.kind === "choose") {
    title = words.addTitle;
    description = words.addDescription;
    dialogActions = [cancel];
    content = (
      <View style={{ gap: 12, paddingBottom: 12 }}>
        <GroupedSection>
          {disks === undefined ? (
            <GroupedSectionItem
              title={words.loadingDisks}
              testID="backups-disks-loading"
              rightElement={<Loading size="sm" accessibilityLabel={words.loadingDisks} />}
            />
          ) : disks === null ? (
            <GroupedSectionItem title={words.disksFailed} titleNumberOfLines={0} testID="backups-disks-failed" />
          ) : disks.length === 0 ? (
            <GroupedSectionItem title={words.noDisks} titleNumberOfLines={0} testID="backups-no-disks" />
          ) : (
            disks.map((disk) => {
              const name = diskName(disk, locale, words);
              // Its size where the disk says it (a played disk does not), and its file system.
              const details =
                disk.size > 0
                  ? words.diskDetails(backupBytes(disk.size, locale, words), fileSystemName(disk.fsType))
                  : fileSystemName(disk.fsType);
              return (
                <GroupedSectionItem
                  key={disk.uuid}
                  icon={<RiHardDrive2Line size="lg" fill={theme.colors.textSecondary} />}
                  title={name}
                  description={disk.added ? `${details} · ${words.alreadyAdded}` : details}
                  disabled={disk.added}
                  onPress={() => toPassword({ kind: "disk", disk, name })}
                  accessibilityLabel={disk.added ? `${name}, ${details}, ${words.alreadyAdded}` : `${name}, ${details}`}
                  testID={`backups-disk-${disk.uuid}`}
                />
              );
            })
          )}
        </GroupedSection>
        <GroupedSection>
          <GroupedSectionItem
            icon={<RiServerLine size="lg" fill={theme.colors.textSecondary} />}
            title={words.networkShare}
            description={words.networkShareDescription}
            onPress={() => {
              setFailure(null);
              setStep({ kind: "share" });
            }}
            testID="backups-network-share"
          />
        </GroupedSection>
      </View>
    );
  } else if (step.kind === "share") {
    title = words.shareTitle;
    description = words.shareDescription;
    const next = () => {
      if (share) toPassword({ kind: "share", share, name: `${share.host}/${share.share}` });
    };
    dialogActions = [
      cancel,
      back({ kind: "choose" }),
      {
        label: words.continue,
        color: "default",
        shouldCloseOnPress: false,
        disabled: share === null,
        onPress: next,
        testID: "backups-share-continue",
      },
    ];
    const input = (
      key: keyof ShareFields,
      label: string,
      testID: string,
      extra?: Partial<React.ComponentProps<typeof TextFieldInput>>,
    ) => (
      <TextField>
        <TextFieldInput
          label={label}
          placeholder={label}
          value={fields[key]}
          onChangeText={field(key)}
          autoCapitalize="none"
          autoCorrect={false}
          returnKeyType="next"
          testID={testID}
          {...extra}
        />
      </TextField>
    );
    content = (
      <View style={{ gap: 8, paddingBottom: 12 }}>
        {input("host", words.host, "backups-share-host", { autoFocus: true, placeholder: `${words.host} (${words.hostPlaceholder})` })}
        {input("share", words.shareName, "backups-share-share")}
        {input("user", words.user, "backups-share-user")}
        <TextField>
          <TextFieldInput
            label={words.sharePassword}
            placeholder={words.sharePassword}
            value={sharePassword}
            onChangeText={(text) => {
              setSharePassword(text);
              setFailure(null);
            }}
            secureTextEntry
            revealable
            autoCapitalize="none"
            autoCorrect={false}
            returnKeyType="next"
            testID="backups-share-password"
          />
        </TextField>
        {input("domain", words.domain, "backups-share-domain", {
          returnKeyType: "done",
          submitBehavior: "submit",
          onSubmitEditing: next,
        })}
        {failureText}
      </View>
    );
  } else {
    const target = step.target;
    title = words.passwordTitle;
    description = words.passwordDescription(target.name);
    dialogActions = [
      cancel,
      back(target.kind === "disk" ? { kind: "choose" } : { kind: "share" }),
      {
        label: target.kind === "disk" ? words.setUpDisk : words.setUpShare,
        color: "default",
        shouldCloseOnPress: false,
        loading: working,
        disabled: problem !== null,
        onPress: () => void setUp(),
        testID: "backups-set-up",
      },
    ];
    const mismatch = problem === "mismatch" && again !== "";
    content = (
      <View style={{ gap: 8, paddingBottom: 12 }}>
        <TextField>
          <TextFieldInput
            label={words.password}
            placeholder={words.password}
            value={password}
            onChangeText={(text) => {
              setPassword(text);
              setFailure(null);
            }}
            secureTextEntry
            autoFocus
            autoCapitalize="none"
            autoCorrect={false}
            editable={!working}
            returnKeyType="next"
            testID="backups-password"
          />
        </TextField>
        <TextFieldHint>{words.passwordRule}</TextFieldHint>
        <TextField invalid={mismatch}>
          <TextFieldInput
            label={words.passwordAgain}
            placeholder={words.passwordAgain}
            value={again}
            onChangeText={(text) => {
              setAgain(text);
              setFailure(null);
            }}
            secureTextEntry
            autoCapitalize="none"
            autoCorrect={false}
            editable={!working}
            returnKeyType="done"
            submitBehavior="submit"
            onSubmitEditing={() => void setUp()}
            testID="backups-password-again"
          />
        </TextField>
        {mismatch ? <TextFieldHint invalid>{words.passwordMismatch}</TextFieldHint> : null}
        <Text style={{ fontSize: 12, lineHeight: 16, color: theme.colors.textSecondary }} testID="backups-password-warning">
          {words.passwordWarning}
        </Text>
        {failureText}
      </View>
    );
  }
  return (
    <Dialog
      open={open}
      onClose={close}
      title={title}
      description={description}
      label={title}
      actions={dialogActions}
      // Nothing dismisses it while a disk is being set up.
      dismissOnBackdrop={!working}
      testID="backups-add-sheet"
    >
      {content}
    </Dialog>
  );
}

export function BackupsPage({ snapshot, actions, strings, locale }: PageProps) {
  const words = strings.backups;
  const theme = useTheme();
  const state = snapshot.backup;
  const act = actions.backup;
  const [adding, setAdding] = React.useState(false);
  const [options, setOptions] = React.useState(false);
  const [asking, setAsking] = React.useState(false);
  const [removing, removeOpen, askRemove, hideRemove] = useHeld<BackupDestination>();
  const [notice, noticeOpen, showNotice, hideNotice] = useHeld<{ title: string; description: string }>();

  if (state === undefined) return null;
  if (state === null) {
    return (
      <GroupedSection>
        <GroupedSectionItem title={words.unavailable} titleNumberOfLines={0} testID="backups-unavailable" />
      </GroupedSection>
    );
  }

  // The words of the moment the session last told: what "Today" is, without a clock of its own.
  const now = Date.now() / 1000;
  const administrator = state.administrator && !!act;
  const running = state.running;
  const ask = async (request: () => Promise<string | null>, title: string) => {
    setAsking(true);
    const failed = await request();
    setAsking(false);
    if (failed) showNotice({ title, description: backupReasonLabel(failed, words) });
  };
  const remove = async (destination: BackupDestination) => {
    if (!act) return;
    const failed = await act.remove(destination.id);
    if (failed) showNotice({ title: words.couldNotRemove(destination.name), description: backupReasonLabel(failed, words) });
  };
  const footer = running
    ? undefined
    : !state.automatic
      ? words.automaticOff
      : state.held
        ? words.held
        : state.nextBackup !== null && state.destinations.length > 0
          ? words.next(backupWhen(state.nextBackup, now, locale, words))
          : undefined;
  const addButton = administrator ? (
    <Button
      size="sm"
      appearance="outline"
      tone="neutral"
      accessibilityLabel={words.add}
      onPress={() => setAdding(true)}
      testID="backups-add"
    >
      {words.add}
    </Button>
  ) : null;
  const empty = state.destinations.length === 0;
  return (
    <View style={PAGE} testID="backups">
      <Progress state={state} locale={locale} words={words} />
      <GroupedSection title={words.disks} footer={footer}>
        {empty ? (
          <GroupedSectionItem
            title={words.disks}
            testID="backups-empty"
            content={
              <View style={{ alignItems: "center", gap: 12, paddingVertical: 16 }}>
                <RiHistoryLine size="3xl" fill={theme.colors.textSecondary} />
                <Text style={{ fontSize: 13, lineHeight: 18, textAlign: "center", color: theme.colors.textSecondary, maxWidth: 420 }}>
                  {administrator ? words.emptyDescription : words.emptyNotAdministrator}
                </Text>
                {addButton}
              </View>
            }
          />
        ) : (
          state.destinations.map((destination) => (
            <DestinationRow
              key={destination.id}
              destination={destination}
              now={now}
              locale={locale}
              words={words}
              onRemove={administrator ? () => askRemove(destination) : null}
            />
          ))
        )}
      </GroupedSection>
      <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
        {empty ? null : addButton}
        <View style={{ flex: 1 }} />
        {administrator ? (
          <Button
            size="sm"
            appearance="outline"
            tone="neutral"
            accessibilityLabel={words.options}
            onPress={() => setOptions(true)}
            testID="backups-options"
          >
            {words.options}
          </Button>
        ) : null}
        {act && running ? (
          <Button
            size="sm"
            appearance="outline"
            tone="neutral"
            accessibilityLabel={words.skip}
            loading={asking}
            onPress={() => void ask(() => act.stop(), words.couldNotSkip)}
            testID="backups-skip"
          >
            {words.skip}
          </Button>
        ) : act ? (
          <Button
            size="sm"
            appearance="outline"
            tone="neutral"
            accessibilityLabel={words.backUpNow}
            loading={asking}
            disabled={!canBackUp(state)}
            onPress={() => void ask(() => act.backUpNow(), words.couldNotStart)}
            testID="backups-now"
          >
            {words.backUpNow}
          </Button>
        ) : null}
      </View>
      {administrator ? (
        <>
          <AddSheet open={adding} onClose={() => setAdding(false)} actions={actions} words={words} locale={locale} />
          <OptionsSheet
            open={options}
            onClose={() => setOptions(false)}
            state={state}
            snapshot={snapshot}
            actions={actions}
            words={words}
          />
        </>
      ) : null}
      <AlertDialog
        open={removeOpen}
        onOpenChange={(open) => {
          if (!open) hideRemove();
        }}
        title={words.removeTitle(removing?.name ?? "")}
        description={removing?.kind === "share" ? words.removeShareDescription : words.removeDiskDescription}
        confirmLabel={words.remove}
        cancelLabel={words.cancel}
        destructive
        defaultAction="cancel"
        onConfirm={() => {
          if (removing) void remove(removing);
        }}
        testID="backups-remove-confirm"
      />
      <AlertDialog
        open={noticeOpen}
        onOpenChange={(open) => {
          if (!open) hideNotice();
        }}
        title={notice?.title ?? ""}
        description={notice?.description}
        confirmLabel={words.ok}
        hideCancel
        testID="backups-failed"
      />
    </View>
  );
}
