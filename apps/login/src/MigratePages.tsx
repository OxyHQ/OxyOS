/**
 * Transfer Your Data, the setup assistant's step on a computer's first account
 * (docs/adr/0059-backups.mdx, decision 7), as a Mac's Setup Assistant offers Migration Assistant
 * before its first user: another computer's accounts brought over from its backup on a disk or a
 * share, or Not Now, to set up as new. Its pages, in the assistant's own frame: where the backups
 * are (a disk, or a network share), the backup password, the computer and the backup (newest
 * first, with the accounts it holds), what migrating brings over, then how far it is. The
 * platform's migration service does the work as root; once it is done the assistant ends, and the
 * login screen lists the accounts brought over, each to sign in again online.
 *
 * The backup password and a share's go to the service and are forgotten here as they are sent.
 */
import React, { useEffect, useRef, useState, type ReactNode } from "react";
import { Pressable, ScrollView, TextInput, View } from "react-native";
import { ChoiceList } from "@oxy.so/bloom/choice-list";
import { Loading } from "@oxy.so/bloom/loading";
import { Meter } from "@oxy.so/bloom/stat-bar";
import { useTheme } from "@oxy.so/bloom/theme";
import { Text } from "@oxy.so/bloom/typography";
import { RiArrowLeftRightLine } from "@oxy.so/bloom/icons/RiArrowLeftRightLine";
import { RiComputerLine } from "@oxy.so/bloom/icons/RiComputerLine";
import { RiErrorWarningLine } from "@oxy.so/bloom/icons/RiErrorWarningLine";
import { RiHardDrive2Line } from "@oxy.so/bloom/icons/RiHardDrive2Line";
import { RiHistoryLine } from "@oxy.so/bloom/icons/RiHistoryLine";
import { RiKey2Line } from "@oxy.so/bloom/icons/RiKey2Line";
import { PasswordField } from "../../shell/src/components/PasswordField";
import type { ShellLock } from "../../shell/src/model";
import { useShellLocale } from "../../shell/src/useShellMessages";
import {
  afterBackups,
  backupDate,
  diskName,
  disksFrom,
  migratedFrom,
  progressFrom,
  shareFrom,
  type BackupSource,
  type MigrateProblem,
  type MigrateStep,
} from "./migrate";
import { useLoginMessages } from "./useLoginMessages";
import type { AssistantPage } from "./SetupAssistant";

/** Migrating from a backup (docs/adr/0059-backups.mdx, decision 7): the platform's migration
    service, which does the work and answers only while no account is enrolled. Each a promise of
    its answer (a failure when it could not be asked); the passwords go to it, never kept here. */
export type MigrationActions = {
  disks: () => Promise<unknown>;
  backups: (source: BackupSource, password: string, sharePassword: string | null) => Promise<unknown>;
  /** `progress` with each of its progress messages, before the answer. */
  migrate: (repository: string, snapshot: string, progress: (told: unknown) => void) => Promise<unknown>;
  /** Ends the conversation: a disk the service mounted for it is let go. */
  end: () => void;
};

/** The accounts a migration brought over. */
export type Migrated = { user: string; name: string }[];

const ICON = 56;

function Icon({ children }: { children: (color: string) => ReactNode }) {
  const theme = useTheme();
  return <View accessible={false}>{children(theme.colors.primary)}</View>;
}

/** A page's message, said as it changes. */
function Message({ text, tone = "quiet" }: { text: string; tone?: "quiet" | "problem" }) {
  const theme = useTheme();
  return (
    <Text
      testID="login-migrate-message"
      accessibilityLiveRegion="polite"
      style={{ fontSize: 15, lineHeight: 22, color: tone === "problem" ? theme.colors.text : theme.colors.textSecondary }}
    >
      {text}
    </Text>
  );
}

/** A text field of the share's page. */
function Field({
  testID,
  label,
  value,
  onChangeText,
  secure,
  onSubmit,
  autoFocus,
}: {
  testID: string;
  label: string;
  value: string;
  onChangeText: (text: string) => void;
  secure?: boolean;
  onSubmit?: () => void;
  autoFocus?: boolean;
}) {
  const theme = useTheme();
  return (
    <TextInput
      testID={testID}
      accessibilityLabel={label}
      value={value}
      onChangeText={onChangeText}
      onSubmitEditing={onSubmit}
      autoFocus={autoFocus}
      secureTextEntry={secure}
      autoCapitalize="none"
      autoCorrect={false}
      placeholder={label}
      placeholderTextColor={theme.colors.textSecondary}
      submitBehavior={onSubmit ? "submit" : "blurAndSubmit"}
      maxLength={256}
      style={{
        height: 44,
        borderRadius: 12,
        paddingHorizontal: 14,
        borderWidth: 1,
        borderColor: theme.colors.border,
        color: theme.colors.text,
        fontSize: 17,
      }}
    />
  );
}

/** Choices of a page (disks, computers, backups): Bloom's `ChoiceList`, upright, the chosen one
    marked; Continue goes on with it. */
function Choices({
  label,
  value,
  choose,
  options,
}: {
  label: string;
  value: string;
  choose: (value: string) => void;
  options: { value: string; label: string; detail?: string; testID: string }[];
}) {
  const theme = useTheme();
  return (
    <ChoiceList
      label={label}
      value={value}
      onValueChange={choose}
      orientation="vertical"
      options={options.map(({ value: one, label: name, testID }) => ({ value: one, label: name, testID }))}
      style={{ gap: 6 }}
      optionStyle={{ borderRadius: 12 }}
      renderContent={(option, { selected }) => {
        const detail = options.find((one) => one.value === option.value)?.detail;
        return (
          <View
            style={{
              paddingHorizontal: 14,
              paddingVertical: 10,
              borderRadius: 12,
              borderWidth: selected ? 2 : 1,
              borderColor: selected ? theme.colors.primary : theme.colors.border,
            }}
          >
            <Text style={{ fontSize: 15, fontWeight: selected ? "600" : "400", color: theme.colors.text }}>{option.label}</Text>
            {detail ? <Text style={{ fontSize: 13, marginTop: 2, color: theme.colors.textSecondary }}>{detail}</Text> : null}
          </View>
        );
      }}
    />
  );
}

/** A link of a page's content (Look Again). */
function Link({ testID, label, onPress }: { testID: string; label: string; onPress: () => void }) {
  const theme = useTheme();
  return (
    <Pressable testID={testID} accessibilityRole="button" accessibilityLabel={label} onPress={onPress} style={{ paddingVertical: 4 }}>
      <Text style={{ fontSize: 15, fontWeight: "600", color: theme.colors.primary }}>{label}</Text>
    </Pressable>
  );
}

/** Transfer Your Data: from a backup, or set up as new. */
export function useTransferPage({ fromBackup, asNew }: { fromBackup: () => void; asNew: () => void }): AssistantPage {
  const words = useLoginMessages().migrate;
  const setupWords = useLoginMessages().setup;
  const [choice, setChoice] = useState<"backup" | "new">("backup");
  return {
    id: "transfer",
    title: words.transferTitle,
    description: words.transferDescription,
    icon: <Icon>{(color) => <RiArrowLeftRightLine width={ICON} height={ICON} fill={color} />}</Icon>,
    content: (
      <Choices
        label={words.transferTitle}
        value={choice}
        choose={(value) => setChoice(value as "backup" | "new")}
        options={[
          { value: "backup", label: words.fromBackup, detail: words.fromBackupDetail, testID: "login-transfer-backup" },
          { value: "new", label: words.asNew, detail: words.asNewDetail, testID: "login-transfer-new" },
        ]}
      />
    ),
    primary: {
      label: setupWords.continue,
      testID: "login-transfer-continue",
      onPress: () => (choice === "backup" ? fromBackup() : asNew()),
    },
  };
}

/**
 * The migration's page for where it stands (`step`), null when it is not migrating. `setStep`
 * moves it; `migrated` is told the accounts brought over once the service is done.
 */
export function useMigratePage({
  migration,
  step,
  setStep,
  migrated,
  capsLock,
  announce,
}: {
  migration: MigrationActions | undefined;
  step: MigrateStep | null;
  setStep: (step: MigrateStep) => void;
  migrated: (accounts: Migrated) => void;
  capsLock?: boolean;
  announce?: (text: string, polite: boolean) => void;
}): AssistantPage | null {
  const catalogue = useLoginMessages();
  const words = catalogue.migrate;
  const setupWords = catalogue.setup;
  const locale = useShellLocale();
  const [busy, setBusy] = useState(false);
  const [answer, setAnswer] = useState<ShellLock["answer"]>(undefined);
  const answered = useRef(0);
  const [disk, setDisk] = useState<string | null>(null);
  const [computer, setComputer] = useState<string | null>(null);
  const [snapshot, setSnapshot] = useState<string | null>(null);
  const [share, setShare] = useState({ host: "", share: "", user: "" });
  const [typedSharePassword, setTypedSharePassword] = useState("");
  // A share's password, until it is sent with the backup password: then forgotten.
  const sharePassword = useRef("");
  const latest = useRef(migration);
  latest.current = migration;

  // The disks, asked as the page that lists them opens (again with Look Again).
  useEffect(() => {
    if (step?.step !== "source" || step.disks !== null || !latest.current) return undefined;
    let current = true;
    latest.current
      .disks()
      .catch(() => null)
      .then((told) => {
        if (!current) return;
        const disks = disksFrom(told);
        const listed = typeof disks === "string" ? [] : disks;
        setDisk((chosen) => (listed.some((one) => one.uuid === chosen) ? chosen : (listed[0]?.uuid ?? null)));
        setStep({ step: "source", disks: listed, problem: typeof disks === "string" ? disks : step.problem });
      });
    return () => {
      current = false;
    };
  }, [step]);

  if (!step || !migration) return null;
  const said = (problem: MigrateProblem | null) =>
    problem ? <Message text={words.problems[problem]} tone="problem" /> : null;
  const names = (accounts: { name: string }[]) => words.list(accounts.map((account) => account.name));
  const id = "migrate";

  switch (step.step) {
    case "source": {
      const disks = step.disks;
      const chosen = disks?.find((one) => one.uuid === disk) ?? disks?.[0];
      return {
        id,
        title: words.sourceTitle,
        description: words.sourceDescription,
        icon: <Icon>{(color) => <RiHardDrive2Line width={ICON} height={ICON} fill={color} />}</Icon>,
        content: (
          <View style={{ gap: 12 }}>
            {disks === null ? (
              <View style={{ alignItems: "center", gap: 12, paddingVertical: 12 }}>
                <Loading size="lg" />
                <Message text={words.lookingForDisks} />
              </View>
            ) : disks.length === 0 ? (
              <Message text={words.noDisks} />
            ) : (
              <Choices
                label={words.disks}
                value={chosen?.uuid ?? ""}
                choose={setDisk}
                options={disks.map((one) => ({
                  value: one.uuid,
                  label: diskName(one, locale),
                  testID: `login-migrate-disk-${one.uuid}`,
                }))}
              />
            )}
            {said(step.problem)}
            {disks !== null && (
              <Link
                testID="login-migrate-look-again"
                label={words.lookAgain}
                onPress={() => setStep({ step: "source", disks: null, problem: null })}
              />
            )}
          </View>
        ),
        primary: {
          label: setupWords.continue,
          testID: "login-migrate-continue",
          disabled: !chosen,
          onPress: () => {
            if (chosen) {
              setStep({ step: "password", source: { disk: { uuid: chosen.uuid } }, place: diskName(chosen, locale), problem: null });
            }
          },
        },
        secondary: {
          label: words.networkShare,
          testID: "login-migrate-share",
          onPress: () => setStep({ step: "share", problem: null }),
        },
      };
    }
    case "share": {
      const typed = shareFrom(share.host, share.share, share.user);
      const connect = () => {
        if (!typed) return;
        sharePassword.current = typedSharePassword;
        setTypedSharePassword("");
        setStep({ step: "password", source: { share: { share: typed } }, place: `smb://${typed.host}/${typed.share}`, problem: null });
      };
      return {
        id,
        title: words.shareTitle,
        icon: <Icon>{(color) => <RiComputerLine width={ICON} height={ICON} fill={color} />}</Icon>,
        initialFocus: "content",
        content: (
          <View style={{ gap: 10 }}>
            <Field testID="login-migrate-share-host" label={words.shareHost} value={share.host} autoFocus onChangeText={(host) => setShare({ ...share, host })} />
            <Field testID="login-migrate-share-name" label={words.shareName} value={share.share} onChangeText={(name) => setShare({ ...share, share: name })} />
            <Field testID="login-migrate-share-user" label={words.shareUser} value={share.user} onChangeText={(user) => setShare({ ...share, user })} />
            <Field
              testID="login-migrate-share-password"
              label={words.sharePassword}
              value={typedSharePassword}
              secure
              onChangeText={setTypedSharePassword}
              onSubmit={connect}
            />
            {said(step.problem)}
          </View>
        ),
        primary: { label: words.connect, testID: "login-migrate-connect", disabled: !typed, onPress: connect },
      };
    }
    case "password": {
      const at = step;
      return {
        id,
        title: words.passwordTitle(step.place),
        description: words.passwordDescription,
        icon: <Icon>{(color) => <RiKey2Line width={ICON} height={ICON} fill={color} />}</Icon>,
        initialFocus: "content",
        content: (
          <View style={{ gap: 12 }}>
            <PasswordField
              testID="login-migrate"
              placeholder={words.passwordPlaceholder}
              accessibilityLabel={words.passwordPlaceholder}
              authenticate={(password) => {
                if (busy) return false;
                setBusy(true);
                const shared = "share" in at.source ? sharePassword.current : null;
                migration
                  .backups(at.source, password, shared)
                  .catch(() => null)
                  .then((told) => {
                    setBusy(false);
                    const next = afterBackups(told, at);
                    const id = ++answered.current;
                    if (next.step === "password") {
                      // The field shaken: the password again.
                      setStep(next);
                      setAnswer({ id, outcome: "rejected", wait: 0 });
                      return;
                    }
                    sharePassword.current = "";
                    setAnswer({ id, outcome: "busy", wait: 0 });
                    if (next.step === "backup") {
                      setComputer(next.computer.repository);
                      setSnapshot(next.computer.backups[0]?.snapshot ?? null);
                    } else if (next.step === "computer") {
                      setComputer(next.computers[0]?.repository ?? null);
                    }
                    // A share that did not answer as one: its details again.
                    setStep(next.step === "source" && "share" in at.source ? { step: "share", problem: next.problem } : next);
                  });
                return true;
              }}
              answer={answer}
              capsLock={capsLock}
              announce={announce}
            />
            {busy ? <Message text={words.opening} /> : said(step.problem)}
          </View>
        ),
      };
    }
    case "computer": {
      const chosen = step.computers.find((one) => one.repository === computer) ?? step.computers[0]!;
      return {
        id,
        title: words.computers,
        icon: <Icon>{(color) => <RiComputerLine width={ICON} height={ICON} fill={color} />}</Icon>,
        content: (
          <Choices
            label={words.computers}
            value={chosen.repository}
            choose={setComputer}
            options={step.computers.map((one) => ({
              value: one.repository,
              label: one.name,
              detail: backupDate(one.backups[0]!.time, locale),
              testID: `login-migrate-computer-${one.repository}`,
            }))}
          />
        ),
        primary: {
          label: setupWords.continue,
          testID: "login-migrate-continue",
          onPress: () => {
            setSnapshot(chosen.backups[0]?.snapshot ?? null);
            setStep({ step: "backup", computer: chosen, several: step.computers });
          },
        },
      };
    }
    case "backup": {
      const chosen = step.computer.backups.find((one) => one.snapshot === snapshot) ?? step.computer.backups[0]!;
      return {
        id,
        title: words.backups(step.computer.name),
        description: step.computer.accounts.length > 0 ? words.accounts(names(step.computer.accounts)) : undefined,
        icon: <Icon>{(color) => <RiHistoryLine width={ICON} height={ICON} fill={color} />}</Icon>,
        content: (
          <ScrollView testID="login-migrate-backups" style={{ maxHeight: 280 }}>
            <Choices
              label={words.backups(step.computer.name)}
              value={chosen.snapshot}
              choose={setSnapshot}
              options={step.computer.backups.map((one) => ({
                value: one.snapshot,
                label: backupDate(one.time, locale),
                testID: `login-migrate-backup-${one.snapshot}`,
              }))}
            />
          </ScrollView>
        ),
        primary: {
          label: setupWords.continue,
          testID: "login-migrate-continue",
          onPress: () => setStep({ step: "confirm", computer: step.computer, backup: chosen, several: step.several }),
        },
      };
    }
    case "confirm": {
      const { computer: chosen, backup } = step;
      return {
        id,
        title: words.confirmTitle(chosen.name),
        description: words.confirmDescription(backupDate(backup.time, locale), names(chosen.accounts)),
        icon: <Icon>{(color) => <RiArrowLeftRightLine width={ICON} height={ICON} fill={color} />}</Icon>,
        content: null,
        primary: {
          label: words.migrateAction,
          testID: "login-migrate-start",
          onPress: () => {
            setStep({ step: "migrating", computer: chosen, progress: { step: "accounts", percent: 0 } });
            migration
              .migrate(chosen.repository, backup.snapshot, (told) => {
                const progress = progressFrom(told);
                if (progress) setStep({ step: "migrating", computer: chosen, progress });
              })
              .catch(() => null)
              .then((told) => {
                const done = migratedFrom(told);
                if (typeof done === "string") setStep({ step: "failed", problem: done });
                else migrated(done.accounts);
              });
          },
        },
      };
    }
    case "migrating": {
      const { progress } = step;
      const now =
        progress.step === "files"
          ? words.steps.files(new Intl.NumberFormat(locale, { style: "percent" }).format(progress.percent))
          : words.steps[progress.step];
      return {
        id,
        title: step.computer.name,
        description: words.migrating,
        content: (
          <View style={{ gap: 12 }}>
            <Meter
              testID="login-migrate-progress"
              role="progressbar"
              value={progress.step === "finishing" ? 1 : progress.percent}
              accessibilityLabel={words.title}
            />
            <Message text={now} />
          </View>
        ),
      };
    }
    case "failed":
      return {
        id,
        title: words.failedTitle,
        icon: <Icon>{(color) => <RiErrorWarningLine width={ICON} height={ICON} fill={color} />}</Icon>,
        content: said(step.problem),
        primary: {
          label: words.startOver,
          testID: "login-migrate-start-over",
          onPress: () => setStep({ step: "source", disks: null, problem: null }),
        },
      };
  }
}
