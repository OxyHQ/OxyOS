import React, { useEffect, useRef, useState, type ComponentType } from "react";
import {
  AccessibilityInfo,
  Pressable,
  ScrollView,
  TextInput,
  useWindowDimensions,
} from "react-native";
import { ChoiceList } from "@oxy.so/bloom/choice-list";
import { Meter } from "@oxy.so/bloom/stat-bar";
import { Text } from "@oxy.so/bloom/typography";
import { Box } from "../../shell/src/primitives";
import { LockBackdrop } from "../../shell/src/components/LockScreen";
import { UserPicture } from "../../shell/src/components/UserPicture";
import { PasswordField, textShadow } from "../../shell/src/components/PasswordField";
import type { ShellLock, ShellWallpaper } from "../../shell/src/model";
import type { MaskedProps } from "../../shell/src/platform";
import { chosenAccount, opensOnSetup, type Accounts, type LoginAccount } from "./model";
import {
  afterEnroll,
  type EnrollAnswer,
  type SetupPlatform,
  type SetupStep,
  type SignedInAccount,
} from "./setup";
import { useLoginMessages } from "./useLoginMessages";
import { useShellMessages } from "../../shell/src/useShellMessages";
import {
  afterBackups,
  backupDate,
  diskName,
  disksFrom,
  migratedFrom,
  progressFrom,
  shareFrom,
  type BackupSource,
  type MigrateStep,
} from "./migrate";

export type { LoginAccount, Accounts } from "./model";

/** The SDK's own sign-in, as the platform runs it for setup: it calls `onSignedIn` once the
    credential a sign-in committed was handed over, and the account has a password. */
export type SetupSignInProps = {
  platform: SetupPlatform;
  onSignedIn: (account: SignedInAccount) => void;
  /** The width the SDK's sign-in asks of its card (its split card, with the QR), padding
      included; null for the card's own. */
  onFrameWidth: (maxWidth: number | null) => void;
};

/** Setting an account up (docs/adr/0020-oxy-account-login.mdx, decision 4): the platform's. */
export type SetupActions = {
  /** Where the SDK's sign-in runs, from the accounts' service; null when it sets nothing up. */
  platform: () => SetupPlatform | null;
  /** The SDK's own sign-in. */
  SignIn: ComponentType<SetupSignInProps>;
  /** The account's password once more, with the credential the sign-in handed over. */
  enroll: (password: string) => Promise<EnrollAnswer>;
  /** The second factor the accounts' service asked for, on the same setup. */
  secondFactor: (code: string) => Promise<EnrollAnswer>;
  /** Ends the setup: what the platform held of it is forgotten. */
  end: () => void;
};

/** Migrate from a Backup (docs/adr/0059-backups.mdx, decision 7): the platform's migration
    service, which does the work and answers only while no account is enrolled. Each a promise of
    its answer ({failed} when it could not be asked); the passwords go to it, never kept here. */
export type MigrationActions = {
  disks: () => Promise<unknown>;
  backups: (source: BackupSource, password: string, sharePassword: string | null) => Promise<unknown>;
  /** `progress` with each of its progress messages, before the answer. */
  migrate: (repository: string, snapshot: string, progress: (told: unknown) => void) => Promise<unknown>;
  /** Ends the conversation: a disk the service mounted for it is let go. */
  end: () => void;
};

/** What the login screen asks of its platform. It holds no authority over who signs in. */
export type LoginActions = {
  /** Sends the password typed for `account` (its Unix user) to be checked; false if it could
      not be sent. */
  authenticate: (account: string, password: string) => boolean;
  /** The latest answer, a new `id` for each. */
  answer?: ShellLock["answer"];
  capsLock?: boolean;
  /** Whether Oxy's API (`api`, else the system's) can be reached now: setup is online only. */
  online: (api?: string) => Promise<boolean>;
  /** Adding an account; none where the platform sets none up. */
  setup?: SetupActions;
  /** Migrating from a backup at the first setup; none where the platform offers none. */
  migration?: MigrationActions;
  /** The accounts again, once one was added. */
  reloadAccounts?: () => void;
  /** Says a text to assistive technologies (the platform's), else React Native's announcement. */
  announce?: (text: string, polite: boolean) => void;
};

const white = (size: number, weight: "400" | "600" = "400", alpha = 1) => ({
  ...textShadow,
  color: alpha === 1 ? "#fff" : `rgba(255,255,255,${alpha})`,
  fontSize: size,
  fontWeight: weight,
});

function Link({
  testID,
  label,
  onPress,
}: {
  testID: string;
  label: string;
  onPress: () => void;
}) {
  return (
    <Pressable
      testID={testID}
      accessibilityRole="button"
      accessibilityLabel={label}
      onPress={onPress}
      style={{ marginTop: 14, paddingHorizontal: 12, paddingVertical: 4 }}
    >
      <Text style={white(13, "600", 0.9)}>{label}</Text>
    </Pressable>
  );
}

/** A setup's message, said as it changes. */
function SetupMessage({ text }: { text: string }) {
  return (
    <Text
      testID="login-setup-message"
      accessibilityRole="alert"
      accessibilityLiveRegion="polite"
      style={{ ...white(13, "400", 0.85), marginTop: 10, textAlign: "center" }}
    >
      {text}
    </Text>
  );
}

/** The second factor the accounts' service asked for: the authenticator's code, said wrong when
    it was, sent with Return or Continue. The code goes to the platform, never kept here. */
function SecondFactor({
  invalid,
  submit,
  busy,
}: {
  invalid: boolean;
  submit: (code: string) => void;
  busy: boolean;
}) {
  const words = useLoginMessages().setup;
  const [code, setCode] = useState("");
  // What the field holds as Return is pressed (`nativeEvent.text`), not the state: typed quickly,
  // Return arrives before the last characters' state is rendered.
  const send = (typed: string) => {
    if (!busy && typed.trim()) {
      submit(typed.trim());
      setCode("");
    }
  };
  return (
    <>
      <TextInput
        testID="login-setup-code"
        accessibilityLabel={words.code}
        value={code}
        onChangeText={setCode}
        onSubmitEditing={({ nativeEvent }) => send(nativeEvent.text)}
        autoFocus
        keyboardType="number-pad"
        autoComplete="one-time-code"
        placeholder={words.code}
        placeholderTextColor="rgba(255,255,255,0.65)"
        submitBehavior="submit"
        maxLength={32}
        style={{
          marginTop: 12,
          width: 200,
          height: 30,
          borderRadius: 15,
          paddingHorizontal: 14,
          backgroundColor: "rgba(255,255,255,0.22)",
          color: "#fff",
          fontSize: 13,
        }}
      />
      {invalid ? <SetupMessage text={words.secondFactorInvalid} /> : null}
      <Link testID="login-setup-continue" label={words.continue} onPress={() => send(code)} />
    </>
  );
}

/**
 * Adding an account (docs/adr/0020-oxy-account-login.mdx, decision 4): online only, as ADR 0020
 * decides; offline it says so and does nothing else. Online, the SDK's own sign-in (the
 * platform's); then the account's password once more and, when Oxy asks, a second factor, which
 * the platform's accounts' service sets the account up with. Enrolled, the new account is chosen.
 */
function SetupPanel({
  step,
  setStep,
  actions,
  enrolled,
  back,
}: {
  step: SetupStep;
  setStep: (step: SetupStep) => void;
  actions: LoginActions;
  enrolled: (user: string, name: string) => void;
  back?: () => void;
}) {
  const words = useLoginMessages().setup;
  const { height } = useWindowDimensions();
  const setup = actions.setup;
  // The card the SDK's sign-in is on: as wide as it asks (its split card, with the QR a phone
  // scans, beside the form), else the setup's own.
  const [frameWidth, setFrameWidth] = useState<number | null>(null);
  const [answer, setAnswer] = useState<ShellLock["answer"]>(undefined);
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const answered = useRef(0);
  const settle = (told: EnrollAnswer, at: "password" | "secondFactor") => {
    setBusy(false);
    const next = afterEnroll(told, at);
    const id = ++answered.current;
    switch (next.next) {
      case "enrolled":
        setProblem(null);
        enrolled(next.user, "account" in step ? step.account.name || step.account.id : "");
        return;
      case "secondFactor":
        if (step.step === "password" || step.step === "secondFactor") {
          setStep({ step: "secondFactor", account: step.account, invalid: next.invalid });
        }
        return;
      case "password":
        if (step.step === "password" || step.step === "secondFactor") {
          setStep({ step: "password", account: step.account });
        }
        setProblem(next.problem === "unusable" ? words.unusablePassword : next.problem === "failed" ? words.failed : null);
        setAnswer({
          id,
          outcome: next.problem === "incorrect" ? "rejected" : next.problem === "throttled" ? "throttled" : "busy",
          wait: next.wait,
        });
        return;
      case "signIn":
        setup?.end();
        setStep({ step: "checking" });
        return;
      case "offline":
        setup?.end();
        setStep({ step: "offline" });
    }
  };
  let content: React.ReactNode;
  if (step.step === "signIn" && setup) {
    const SignIn = setup.SignIn;
    // The SDK's sign-in on a card of its own over the dimmed wallpaper, as a Mac's setup shows its
    // sign-in: its words in its own colours, legible whatever the wallpaper. The card is the
    // SDK's surface, as an app's account dialog is: it grows to the width the sign-in asks.
    content = (
      <Box
        className="mt-4 overflow-hidden"
        style={{
          width: "100%",
          backgroundColor: "rgba(18,18,22,0.88)",
          borderRadius: 24,
        }}
      >
        {/* Taller than the screen (a code under a form), it scrolls within its card. */}
        <ScrollView
          testID="login-setup-card"
          style={{ maxHeight: Math.max(240, height - 150) }}
          contentContainerStyle={{ paddingHorizontal: 32, paddingVertical: 28 }}
        >
          <SignIn
            platform={step.platform}
            onSignedIn={(account) => setStep({ step: "password", account })}
            onFrameWidth={setFrameWidth}
          />
        </ScrollView>
      </Box>
    );
  } else if (step.step === "password" && setup) {
    content = (
      <>
        <Text style={{ ...white(15, "600"), marginTop: 10, textAlign: "center" }}>
          {words.passwordTitle(step.account.name || step.account.id)}
        </Text>
        <Text style={{ ...white(13, "400", 0.85), marginTop: 6, textAlign: "center" }}>
          {words.passwordDescription}
        </Text>
        <PasswordField
          testID="login-setup"
          placeholder={words.passwordPlaceholder}
          accessibilityLabel={words.passwordPlaceholder}
          authenticate={(password) => {
            setProblem(null);
            setBusy(true);
            setup.enroll(password).then((told) => settle(told, "password"));
            return true;
          }}
          answer={answer}
          capsLock={actions.capsLock}
          announce={actions.announce}
        />
        {problem ? <SetupMessage text={problem} /> : null}
      </>
    );
  } else if (step.step === "secondFactor" && setup) {
    content = (
      <>
        <Text style={{ ...white(15, "600"), marginTop: 10, textAlign: "center" }}>{words.secondFactorTitle}</Text>
        <SecondFactor
          invalid={step.invalid}
          busy={busy}
          submit={(code) => {
            setBusy(true);
            setup.secondFactor(code).then((told) => settle(told, "secondFactor"));
          }}
        />
        {busy ? <SetupMessage text={words.adding} /> : null}
      </>
    );
  } else {
    const message =
      step.step === "checking" ? words.checking : step.step === "offline" ? words.offline : words.notConfigured;
    content = (
      <>
        <SetupMessage text={message} />
        {step.step !== "checking" && (
          <Link testID="login-setup-retry" label={words.retry} onPress={() => setStep({ step: "checking" })} />
        )}
      </>
    );
  }
  return (
    <Box
      testID="login-setup"
      className="items-center"
      style={step.step === "signIn" ? { width: frameWidth ?? 512, maxWidth: "100%" } : { maxWidth: 360 }}
    >
      <Text style={white(17, "600")}>{words.title}</Text>
      {content}
      {back && (
        <Link
          testID="login-setup-back"
          label={words.back}
          onPress={() => {
            setup?.end();
            back();
          }}
        />
      )}
    </Box>
  );
}

/** A text field of the panel's own, as the second factor's code is typed. */
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
      placeholderTextColor="rgba(255,255,255,0.65)"
      submitBehavior={onSubmit ? "submit" : "blurAndSubmit"}
      maxLength={256}
      style={{
        marginTop: 10,
        width: 240,
        height: 30,
        borderRadius: 15,
        paddingHorizontal: 14,
        backgroundColor: "rgba(255,255,255,0.22)",
        color: "#fff",
        fontSize: 13,
      }}
    />
  );
}

/** Choices of the panel's own (disks, computers, backups): Bloom's `ChoiceList`, upright, each
    chosen with a press, Space or Return, going on with it. */
function Choices({
  label,
  options,
  choose,
}: {
  label: string;
  options: { value: string; label: string; detail?: string; testID: string }[];
  choose: (value: string) => void;
}) {
  if (options.length === 0) return null;
  return (
    <ChoiceList
      label={label}
      value={options[0]!.value}
      onValueChange={choose}
      orientation="vertical"
      options={options.map(({ value, label: name, testID }) => ({ value, label: name, testID }))}
      style={{ gap: 2, marginTop: 12, alignSelf: "stretch" }}
      optionStyle={{ paddingHorizontal: 14, paddingVertical: 8, borderRadius: 10, backgroundColor: "rgba(255,255,255,0.12)" }}
      renderContent={(option) => {
        const detail = options.find((one) => one.value === option.value)?.detail;
        return (
          <Box>
            <Text style={white(13, "600")}>{option.label}</Text>
            {detail ? <Text style={{ ...white(12, "400", 0.8), marginTop: 2 }}>{detail}</Text> : null}
          </Box>
        );
      }}
    />
  );
}

/**
 * Migrate from a Backup (docs/adr/0059-backups.mdx, decision 7): at the first setup, another
 * computer's accounts brought over from its backup on a disk or a share, as a Mac's Migration
 * Assistant brings them from Time Machine. Where the backups are, their password, the computer and
 * the backup (newest first), then the migration and how far it is. The platform's migration
 * service does the work; once it is done the accounts are listed, each to sign in again online.
 */
function MigratePanel({
  step,
  setStep,
  migration,
  migrated,
  back,
  announce,
}: {
  step: MigrateStep;
  setStep: (step: MigrateStep) => void;
  migration: MigrationActions;
  migrated: (count: number) => void;
  back: () => void;
  announce?: (text: string, polite: boolean) => void;
}) {
  const catalogue = useLoginMessages();
  const words = catalogue.migrate;
  const { language } = useShellMessages();
  const [busy, setBusy] = useState(false);
  const [answer, setAnswer] = useState<ShellLock["answer"]>(undefined);
  const answered = useRef(0);
  const [share, setShare] = useState({ host: "", share: "", user: "" });
  // A share's password, until the backups' password is sent with it: then forgotten.
  const sharePassword = useRef("");
  const [typedSharePassword, setTypedSharePassword] = useState("");
  const latest = useRef(migration);
  latest.current = migration;
  // The disks, asked for as the step that lists them opens (again with Look Again).
  useEffect(() => {
    if (step.step !== "source" || step.disks !== null) return undefined;
    let current = true;
    latest.current
      .disks()
      .catch(() => null)
      .then((told) => {
        if (!current) return;
        const disks = disksFrom(told);
        setStep({ step: "source", disks: typeof disks === "string" ? [] : disks, problem: typeof disks === "string" ? disks : step.problem });
      });
    return () => {
      current = false;
    };
  }, [step]);
  const names = (accounts: { name: string }[]) => words.list(accounts.map((account) => account.name));
  const problem = (which: string | null) => (which ? <SetupMessage text={words.problems[which as keyof typeof words.problems]} /> : null);
  let content: React.ReactNode;
  let goBack: (() => void) | null = null;
  switch (step.step) {
    case "source": {
      goBack = back;
      content = (
        <>
          <Text style={{ ...white(13, "400", 0.85), marginTop: 6, textAlign: "center" }}>{words.sourceDescription}</Text>
          {step.disks === null ? (
            <SetupMessage text={words.lookingForDisks} />
          ) : step.disks.length === 0 ? (
            <SetupMessage text={words.noDisks} />
          ) : (
            <Choices
              label={words.disks}
              options={step.disks.map((disk) => ({
                value: disk.uuid,
                label: diskName(disk, language),
                testID: `login-migrate-disk-${disk.uuid}`,
              }))}
              choose={(uuid) => {
                const disk = step.disks?.find((one) => one.uuid === uuid);
                if (disk) {
                  setStep({ step: "password", source: { disk: { uuid } }, place: diskName(disk, language), problem: null });
                }
              }}
            />
          )}
          {problem(step.problem)}
          <Link testID="login-migrate-share" label={words.networkShare} onPress={() => setStep({ step: "share", problem: null })} />
          {step.disks !== null && (
            <Link testID="login-migrate-look-again" label={words.lookAgain} onPress={() => setStep({ step: "source", disks: null, problem: null })} />
          )}
        </>
      );
      break;
    }
    case "share": {
      goBack = () => setStep({ step: "source", disks: null, problem: null });
      const typed = shareFrom(share.host, share.share, share.user);
      const connect = () => {
        if (!typed) return;
        sharePassword.current = typedSharePassword;
        setTypedSharePassword("");
        setStep({ step: "password", source: { share: { share: typed } }, place: `smb://${typed.host}/${typed.share}`, problem: null });
      };
      content = (
        <>
          <Text style={{ ...white(15, "600"), marginTop: 10, textAlign: "center" }}>{words.shareTitle}</Text>
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
          {problem(step.problem)}
          {typed && <Link testID="login-migrate-connect" label={words.connect} onPress={connect} />}
        </>
      );
      break;
    }
    case "password": {
      goBack = () => {
        sharePassword.current = "";
        setStep({ step: "source", disks: null, problem: null });
      };
      content = (
        <>
          <Text style={{ ...white(15, "600"), marginTop: 10, textAlign: "center" }}>{words.passwordTitle(step.place)}</Text>
          <Text style={{ ...white(13, "400", 0.85), marginTop: 6, textAlign: "center" }}>{words.passwordDescription}</Text>
          <PasswordField
            testID="login-migrate"
            placeholder={words.passwordPlaceholder}
            accessibilityLabel={words.passwordPlaceholder}
            authenticate={(password) => {
              setBusy(true);
              const shared = "share" in step.source ? sharePassword.current : null;
              migration
                .backups(step.source, password, shared)
                .catch(() => null)
                .then((told) => {
                  setBusy(false);
                  const next = afterBackups(told, step);
                  const id = ++answered.current;
                  if (next.step === "password") {
                    setStep(next);
                    setAnswer({ id, outcome: "rejected", wait: 0 });
                    return;
                  }
                  sharePassword.current = "";
                  setAnswer({ id, outcome: "busy", wait: 0 });
                  // A share that did not answer as one: its details again.
                  if (next.step === "source" && "share" in step.source) {
                    setStep({ step: "share", problem: next.problem });
                  } else {
                    setStep(next);
                  }
                });
              return true;
            }}
            answer={answer}
            announce={announce}
          />
          {busy ? <SetupMessage text={words.opening} /> : problem(step.problem)}
        </>
      );
      break;
    }
    case "computer": {
      goBack = () => setStep({ step: "source", disks: null, problem: null });
      content = (
        <Choices
          label={words.computers}
          options={step.computers.map((computer) => ({
            value: computer.repository,
            label: computer.name,
            detail: backupDate(computer.backups[0]!.time, language),
            testID: `login-migrate-computer-${computer.repository}`,
          }))}
          choose={(repository) => {
            const computer = step.computers.find((one) => one.repository === repository);
            if (computer) setStep({ step: "backup", computer, several: step.computers });
          }}
        />
      );
      break;
    }
    case "backup": {
      goBack = () =>
        setStep(step.several ? { step: "computer", computers: step.several } : { step: "source", disks: null, problem: null });
      content = (
        <>
          <Text style={{ ...white(15, "600"), marginTop: 10, textAlign: "center" }}>{words.backups(step.computer.name)}</Text>
          {step.computer.accounts.length > 0 && (
            <Text style={{ ...white(13, "400", 0.85), marginTop: 6, textAlign: "center" }}>
              {words.accounts(names(step.computer.accounts))}
            </Text>
          )}
          <Box style={{ alignSelf: "stretch", maxHeight: 260 }}>
            <ScrollView testID="login-migrate-backups">
              <Choices
                label={words.backups(step.computer.name)}
                options={step.computer.backups.map((backup) => ({
                  value: backup.snapshot,
                  label: backupDate(backup.time, language),
                  testID: `login-migrate-backup-${backup.snapshot}`,
                }))}
                choose={(snapshot) => {
                  const backup = step.computer.backups.find((one) => one.snapshot === snapshot);
                  if (backup) setStep({ step: "confirm", computer: step.computer, backup, several: step.several });
                }}
              />
            </ScrollView>
          </Box>
        </>
      );
      break;
    }
    case "confirm": {
      goBack = () => setStep({ step: "backup", computer: step.computer, several: step.several });
      const start = () => {
        const { computer, backup } = step;
        setStep({ step: "migrating", computer, progress: { step: "accounts", percent: 0 } });
        migration
          .migrate(computer.repository, backup.snapshot, (told) => {
            const progress = progressFrom(told);
            if (progress) setStep({ step: "migrating", computer, progress });
          })
          .catch(() => null)
          .then((told) => {
            const done = migratedFrom(told);
            if (typeof done === "string") setStep({ step: "failed", problem: done });
            else migrated(done.accounts.length);
          });
      };
      content = (
        <>
          <Text style={{ ...white(15, "600"), marginTop: 10, textAlign: "center" }}>{words.confirmTitle(step.computer.name)}</Text>
          <Text style={{ ...white(13, "400", 0.85), marginTop: 6, textAlign: "center" }}>
            {words.confirmDescription(backupDate(step.backup.time, language), names(step.computer.accounts))}
          </Text>
          <Link testID="login-migrate-start" label={words.migrateAction} onPress={start} />
        </>
      );
      break;
    }
    case "migrating": {
      const { progress } = step;
      const said =
        progress.step === "files"
          ? words.steps.files(new Intl.NumberFormat(language, { style: "percent" }).format(progress.percent))
          : words.steps[progress.step];
      content = (
        <>
          <Text style={{ ...white(15, "600"), marginTop: 10, textAlign: "center" }}>{step.computer.name}</Text>
          <Box style={{ marginTop: 14 }}>
            <Meter
              testID="login-migrate-progress"
              role="progressbar"
              value={progress.step === "finishing" ? 1 : progress.percent}
              accessibilityLabel={words.title}
              width={240}
            />
          </Box>
          <SetupMessage text={said} />
          <Text style={{ ...white(12, "400", 0.75), marginTop: 6, textAlign: "center" }}>{words.migrating}</Text>
        </>
      );
      break;
    }
    case "failed": {
      goBack = back;
      content = (
        <>
          {problem(step.problem)}
          <Link testID="login-migrate-start-over" label={words.startOver} onPress={() => setStep({ step: "source", disks: null, problem: null })} />
        </>
      );
      break;
    }
  }
  return (
    <Box testID="login-migrate" className="items-center" style={{ width: 360, maxWidth: "100%" }}>
      <Text style={white(17, "600")}>{words.title}</Text>
      {content}
      {goBack && <Link testID="login-migrate-back" label={catalogue.setup.back} onPress={goBack} />}
    </Box>
  );
}

/** The enrolled accounts to choose from, when there are several: each its picture and name, the
    chosen one's bright. Bloom's `ChoiceList`, as a Mac's login window: one keyboard stop, the
    arrows moving between the accounts without choosing, Space or Return choosing the one whose
    password is asked for (its field then takes the focus); each said once with its place
    ("Bea Ramos, 2 de 3"). */
function AccountChooser({
  accounts,
  chosen,
  choose,
}: {
  accounts: LoginAccount[];
  chosen: string;
  choose: (user: string) => void;
}) {
  const words = useLoginMessages();
  return (
    <ChoiceList
      label={words.accounts}
      value={chosen}
      onValueChange={choose}
      options={accounts.map((account) => ({
        value: account.user,
        label: account.name,
        testID: `login-account-${account.user}`,
      }))}
      style={{ gap: 28 }}
      optionStyle={{ paddingHorizontal: 6, paddingVertical: 4, borderRadius: 12 }}
      renderContent={(option, { selected }) => {
        const account = accounts.find((one) => one.user === option.value);
        return (
          <Box className="items-center" style={{ opacity: selected ? 1 : 0.6 }}>
            <UserPicture picture={account?.picture} />
            <Text style={{ ...white(15, selected ? "600" : "400"), marginTop: 10 }}>
              {option.label}
            </Text>
          </Box>
        );
      }}
    />
  );
}

/**
 * The login screen (docs/adr/0020-oxy-account-login.mdx, decision 4), the lock screen's look
 * before anyone has signed in, as macOS's login window: the wallpaper dimmed, the date and the
 * time, the enrolled accounts (picture and name) and a password field for the one chosen, and
 * "Añadir cuenta". It decides nothing: the password goes through the compositor to the login
 * program, which checks it with PAM and starts the person's session.
 */
export function LoginScreen({
  accounts,
  actions,
  wallpaper,
  now,
  Masked,
}: {
  accounts: Accounts;
  actions: LoginActions;
  wallpaper: ShellWallpaper;
  now: Date;
  Masked?: ComponentType<MaskedProps>;
}) {
  const words = useLoginMessages();
  const [chosen, setChosen] = useState<string | null>(null);
  // Choosing the account already chosen goes on to its password, as Return on a Mac's chosen
  // account does: its field takes the focus again.
  const [toPassword, setToPassword] = useState(0);
  // Setup: closed (null), or where it stands.
  const [setup, setSetup] = useState<SetupStep | null>(null);
  // Migrate from a Backup: closed (null), or where it stands.
  const [migrating, setMigrating] = useState<MigrateStep | null>(null);
  // The first boot: no account yet, so setup is what the screen offers.
  const first = opensOnSetup(accounts);
  const shown: SetupStep | null = setup ?? (first ? { step: "checking" } : null);
  // The platform's checks, as they are now: a new render does not check again.
  const latest = useRef(actions);
  latest.current = actions;
  useEffect(() => {
    if (shown?.step !== "checking") return undefined;
    let current = true;
    const platform = latest.current.setup?.platform() ?? null;
    latest.current
      .online(platform?.api)
      .catch(() => false)
      .then((online) => {
        if (!current) return;
        setSetup(!online ? { step: "offline" } : platform ? { step: "signIn", platform } : { step: "notConfigured" });
      });
    return () => {
      current = false;
    };
  }, [shown?.step]);
  const account = chosenAccount(accounts, chosen);
  let content: React.ReactNode;
  if (accounts === null) {
    content = (
      <Text testID="login-unavailable" accessibilityRole="alert" style={white(13, "400", 0.85)}>
        {words.unreadable}
      </Text>
    );
  } else if (migrating && actions.migration) {
    const migration = actions.migration;
    content = (
      <MigratePanel
        step={migrating}
        setStep={setMigrating}
        migration={migration}
        announce={actions.announce}
        migrated={(count) => {
          // Said once, as the accounts brought over are listed, each to sign in again.
          const said = words.migrate.migrated(count);
          if (actions.announce) actions.announce(said, false);
          else AccessibilityInfo.announceForAccessibility(said);
          migration.end();
          actions.reloadAccounts?.();
          setMigrating(null);
          setSetup(null);
        }}
        back={() => {
          migration.end();
          setMigrating(null);
        }}
      />
    );
  } else if (shown) {
    // At the first setup, before an account is set up, another computer's may be brought over.
    const offerMigration =
      first &&
      actions.migration &&
      (shown.step === "checking" || shown.step === "offline" || shown.step === "notConfigured" || shown.step === "signIn");
    content = (
      <>
        <SetupPanel
          step={shown}
          setStep={setSetup}
          actions={actions}
          enrolled={(user, name) => {
            // Said once, as the new account is listed and chosen, its password field focused.
            const added = words.setup.added(name);
            if (actions.announce) actions.announce(added, false);
            else AccessibilityInfo.announceForAccessibility(added);
            actions.setup?.end();
            actions.reloadAccounts?.();
            setChosen(user);
            setSetup(null);
          }}
          back={account ? () => setSetup(null) : undefined}
        />
        {offerMigration && (
          <Link
            testID="login-migrate-offer"
            label={words.migrate.offer}
            onPress={() => {
              actions.setup?.end();
              setMigrating({ step: "source", disks: null, problem: null });
            }}
          />
        )}
      </>
    );
  } else if (account?.signInAgain) {
    // Signed out by Oxy, or brought over from a backup: its password opens it once it is set up
    // again online, so the screen offers that, never a password that cannot open it.
    content = (
      <>
        {accounts.length > 1 ? (
          <AccountChooser accounts={accounts} chosen={account.user} choose={setChosen} />
        ) : (
          <>
            <UserPicture picture={account.picture} />
            <Text style={{ ...white(15, "600"), marginTop: 10 }}>{account.name}</Text>
          </>
        )}
        <Text
          testID="login-sign-in-again"
          style={{ ...white(13, "400", 0.85), marginTop: 14, maxWidth: 320, textAlign: "center" }}
        >
          {words.signInAgain.description(account.name)}
        </Text>
        <Link testID="login-sign-in-again-action" label={words.signInAgain.action} onPress={() => setSetup({ step: "checking" })} />
      </>
    );
  } else if (account) {
    content = (
      <>
        {/* Several accounts side by side, the chosen one's field under them; one alone, as
            the lock screen shows its user. */}
        {accounts.length > 1 ? (
          <AccountChooser
            accounts={accounts}
            chosen={account.user}
            choose={(user) => {
              if (user === account.user) setToPassword((count) => count + 1);
              else setChosen(user);
            }}
          />
        ) : (
          <>
            <UserPicture picture={account.picture} />
            <Text style={{ ...white(15, "600"), marginTop: 10 }}>{account.name}</Text>
          </>
        )}
        {/* A field of its own for each account: what was typed for one is never sent for
            another. */}
        <PasswordField
          key={account.user}
          testID="login"
          authenticate={(password) => actions.authenticate(account.user, password)}
          answer={actions.answer}
          capsLock={actions.capsLock}
          announce={actions.announce}
          focusRequest={toPassword}
        />
        <Link testID="login-add" label={words.addAccount} onPress={() => setSetup({ step: "checking" })} />
      </>
    );
  }
  return (
    <LockBackdrop
      testID="login"
      wallpaper={wallpaper}
      now={now}
      Masked={Masked}
      clock={migrating ? migrating.step === "source" || migrating.step === "migrating" : shown?.step !== "signIn"}
    >
      {content}
    </LockBackdrop>
  );
}
