/**
 * The setup assistant (docs/adr/0020-oxy-account-login.mdx, decision 7): adding an Oxy account at
 * the login screen as a Mac's Setup Assistant adds its first user, one page at a time in one
 * frame. The frame is the Oxy SDK's (`OxyAssistant` of `@oxy.so/services`, which the platform
 * injects as `SetupActions.Assistant`), so the pages OxyOS adds and the SDK's own sign-in (the QR
 * a phone scans, or a way typed in) look and behave as one: the same header, the same Back and
 * Continue, focus moved and each page said as it comes. What each page shows and where each
 * answer leads is here; where the setup stands is `setup.ts`'s.
 *
 * What the person gives is held only until it is used: the password typed in the sign-in or asked
 * once, and the PIN chosen, go to the accounts' service with the setup, then once more to the
 * login program to sign the new account in (straight to its desktop, never asked again), and are
 * forgotten. A failure keeps them, so Try Again needs nothing typed again.
 *
 * On the computer's first account, where the platform migrates, Transfer Your Data comes after the
 * welcome (MigratePages.tsx, docs/adr/0059-backups.mdx): another computer's accounts brought over
 * from its backup end the assistant, the login screen listing them to sign in again; Not Now goes
 * on to the sign-in.
 */
import React, { useEffect, useRef, useState, type ComponentType, type ReactNode } from "react";
import { TextInput, View } from "react-native";
import { ChoiceList } from "@oxy.so/bloom/choice-list";
import { Loading } from "@oxy.so/bloom/loading";
import { useTheme } from "@oxy.so/bloom/theme";
import { Text } from "@oxy.so/bloom/typography";
import { RiCheckLine } from "@oxy.so/bloom/icons/RiCheckLine";
import { RiComputerLine } from "@oxy.so/bloom/icons/RiComputerLine";
import { RiErrorWarningLine } from "@oxy.so/bloom/icons/RiErrorWarningLine";
import { RiKey2Line } from "@oxy.so/bloom/icons/RiKey2Line";
import { RiPaletteLine } from "@oxy.so/bloom/icons/RiPaletteLine";
import { RiShieldCheckLine } from "@oxy.so/bloom/icons/RiShieldCheckLine";
import { RiWifiOffLine } from "@oxy.so/bloom/icons/RiWifiOffLine";
import { PasswordField } from "../../shell/src/components/PasswordField";
import type { ShellLock } from "../../shell/src/model";
import {
  afterEnroll,
  APPEARANCES,
  asksPassword,
  backFrom,
  pageOf,
  pagesFor,
  pinOptional,
  SETUP_STAGES,
  secondsLeft,
  usablePin,
  waitFrom,
  type Appearance,
  type EnrollAnswer,
  type SetupPlatform,
  type SetupStage,
  type SetupStep,
  type SignedInAccount,
} from "./setup";
import type { Accounts } from "./model";
import type { MigrateStep } from "./migrate";
import { useMigratePage, useTransferPage, type Migrated, type MigrationActions } from "./MigratePages";
import { useLoginMessages } from "./useLoginMessages";
import { useShellLocale } from "../../shell/src/useShellMessages";

/** A button of a page: Continue (the primary) or one beside it (Not Now). */
export type AssistantAction = { label: string; onPress: () => void; disabled?: boolean; busy?: boolean; testID?: string };

/** A page of the assistant, as the SDK's frame takes it (`OxyAssistantStep`). */
export type AssistantPage = {
  id: string;
  title?: string;
  description?: string;
  icon?: ReactNode;
  content: ReactNode;
  primary?: AssistantAction;
  secondary?: AssistantAction;
  /** Where focus goes as it is shown: its heading, or the field its content focuses itself. */
  initialFocus?: "heading" | "content";
  /** The SDK's own sign-in, in place of `content`. */
  signIn?: true;
};

/** The SDK's sign-in as the assistant's sign-in page reports it: once the credential a sign-in
    committed was handed over, with how it signed in and, for a password typed in it, that
    password (taken as it was typed, never asked again). */
export type SignedIn = (account: SignedInAccount, password?: string) => void;

/** The frame the platform injects: the SDK's assistant, its sign-in page the SDK's chooser in
    the SDK's provider for `platform`. */
export type SetupAssistantFrameProps = {
  label: string;
  pages: AssistantPage[];
  current: string;
  onBack?: () => void;
  backLabel: string;
  /** Where the SDK's sign-in runs; null where the machine sets no account up (its pages then
      need no SDK provider). */
  platform: SetupPlatform | null;
  /** The language the frame's own words speak: the system's (ADR 0040). */
  locale: string;
  onSignedIn: SignedIn;
};

/** What setting an account up asks of the accounts' service, through the platform. */
/** What the accounts' service tells while it sets the account up, before its answer. */
export type SetupProgress = {
  /** Each stage as it starts. */
  stage: (stage: SetupStage) => void;
  /** Oxy asked it to wait (`retryAfterSeconds`) before asking again; it goes on by itself. */
  waiting: (retryAfterSeconds: number) => void;
};

export type EnrollRequest = { password: string | null; pin: string | null; appearance: Appearance };

/** Setting an account up (docs/adr/0020-oxy-account-login.mdx): the platform's. */
export type SetupActions = {
  /** Where the SDK's sign-in runs, from the accounts' service; null when it sets nothing up. */
  platform: () => SetupPlatform | null;
  /** The SDK's assistant frame, with its sign-in. */
  Assistant: ComponentType<SetupAssistantFrameProps>;
  /** Sets the account up with the credential the sign-in handed over, reporting what the
      accounts' service tells as it goes (`progress`). */
  enroll: (request: EnrollRequest, progress: SetupProgress) => Promise<EnrollAnswer>;
  /** The second factor the accounts' service asked for, on the same setup. */
  secondFactor: (code: string, progress: SetupProgress) => Promise<EnrollAnswer>;
  /** The setup again after it failed (`unreachable`, `machine`), with what the accounts' service
      kept of it: the sign-in, never a new one. */
  retry: (progress: SetupProgress) => Promise<EnrollAnswer>;
  /** Ends the setup: what the platform held of it is forgotten. */
  end: () => void;
};

/** What the assistant asks of the login screen around it. */
export type AssistantHost = {
  setup: SetupActions;
  /** Whether Oxy's API can be reached now: setup is online only. */
  online: (api?: string) => Promise<boolean>;
  /** Signs the new account in with what was just chosen; false if it could not be sent. */
  signIn: (user: string, secret: { pin: string } | { password: string }) => boolean;
  /** The login program's latest answer, a new `id` for each. */
  answer?: ShellLock["answer"];
  capsLock?: boolean;
  announce?: (text: string, polite: boolean) => void;
  /** The accounts again, once one was added. */
  reloadAccounts?: () => void;
  /** Migrating from a backup, at the first account's Transfer Your Data; none where the
      platform migrates nothing. */
  migration?: MigrationActions;
  /** Told the accounts a migration brought over: the assistant ends, they are listed. */
  migrated?: (accounts: Migrated) => void;
};

/** The secrets and choices the setup holds until they are used. */
type Draft = {
  password: string | null;
  /** The PIN chosen; null: none for now ("Not Now"); undefined: not chosen yet. */
  pin: string | null | undefined;
  appearance: Appearance | undefined;
};

const ICON = 56;

function PageIcon({ children }: { children: (color: string) => ReactNode }) {
  const theme = useTheme();
  return <View accessible={false}>{children(theme.colors.primary)}</View>;
}

/** A page's message, said as it changes. */
function Message({ text, testID = "login-setup-message", tone = "quiet" }: { text: string; testID?: string; tone?: "quiet" | "problem" }) {
  const theme = useTheme();
  return (
    <Text
      testID={testID}
      // Said once as it changes (a live region; an alert role too would say it twice).
      accessibilityLiveRegion="polite"
      style={{ fontSize: 15, lineHeight: 22, color: tone === "problem" ? theme.colors.text : theme.colors.textSecondary }}
    >
      {text}
    </Text>
  );
}

/** Oxy asked for a moment: the seconds left, counted down, under the stages. Nothing failed and
    nothing is asked of the person. Said once as it begins (its label stays what it first said,
    so a reader does not repeat it each second). */
function Waiting({ until }: { until: number }) {
  const words = useLoginMessages().setup;
  const theme = useTheme();
  const [now, setNow] = useState(() => Date.now());
  const [first] = useState(() => secondsLeft(until, Date.now()));
  useEffect(() => {
    const tick = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(tick);
  }, []);
  return (
    <Text
      testID="login-setup-waiting"
      accessibilityLiveRegion="polite"
      accessibilityLabel={words.waiting(first)}
      style={{ fontSize: 15, lineHeight: 22, color: theme.colors.textSecondary }}
    >
      {words.waiting(secondsLeft(until, now))}
    </Text>
  );
}

/** The second factor's code, sent with Return or Continue. */
function CodeField({ code, setCode, send }: { code: string; setCode: (code: string) => void; send: (code: string) => void }) {
  const words = useLoginMessages().setup;
  const theme = useTheme();
  return (
    <TextInput
      testID="login-setup-code"
      accessibilityLabel={words.code}
      value={code}
      onChangeText={setCode}
      // What the field holds as Return is pressed: typed quickly, Return arrives before the last
      // characters' state is rendered.
      onSubmitEditing={({ nativeEvent }) => send(nativeEvent.text)}
      autoFocus
      keyboardType="number-pad"
      autoComplete="one-time-code"
      placeholder={words.code}
      placeholderTextColor={theme.colors.textSecondary}
      submitBehavior="submit"
      maxLength={32}
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

/** A drawing of an appearance: a window on a desktop, light, dark, or each half. */
function AppearancePreview({ appearance }: { appearance: Appearance }) {
  const half = (dark: boolean) => (
    <View style={{ flex: 1, backgroundColor: dark ? "#1c1c22" : "#eceef2", padding: 8 }}>
      <View style={{ height: 8, width: "70%", borderRadius: 4, backgroundColor: dark ? "#3a3a44" : "#c9ccd4" }} />
      <View style={{ height: 8, width: "45%", marginTop: 6, borderRadius: 4, backgroundColor: dark ? "#2c2c34" : "#d9dbe1" }} />
    </View>
  );
  return (
    <View
      accessible={false}
      style={{ width: 112, height: 72, borderRadius: 10, overflow: "hidden", flexDirection: "row", borderWidth: 1, borderColor: "rgba(127,127,127,0.35)" }}
    >
      {appearance === "light" ? half(false) : appearance === "dark" ? half(true) : [half(false), half(true)].map((side, i) => <React.Fragment key={i}>{side}</React.Fragment>)}
    </View>
  );
}

/** The stages of setting up, each done, under way, or still to come. */
function Stages({ stage, pin }: { stage: SetupStage | "done" | null; pin: boolean }) {
  const words = useLoginMessages().setup;
  const theme = useTheme();
  // The PIN's stage only where one was chosen: the service skips it otherwise.
  const stages = SETUP_STAGES.filter((name) => pin || name !== "securing");
  const reached = stage === "done" ? stages.length : stage ? stages.indexOf(stage) : -1;
  return (
    <View testID="login-setup-stages" style={{ gap: 12 }}>
      {stages.map((name, index) => {
        const done = index < reached;
        const now = index === reached;
        const label = words.stages[name];
        return (
          <View
            key={name}
            testID={`login-setup-stage-${name}`}
            accessible
            accessibilityLabel={done ? `${label}, ${words.stageDone}` : now ? `${label}, ${words.stageNow}` : label}
            style={{ flexDirection: "row", alignItems: "center", gap: 12, opacity: done || now ? 1 : 0.45 }}
          >
            <View style={{ width: 20, height: 20, alignItems: "center", justifyContent: "center" }}>
              {done ? (
                <RiCheckLine width={20} height={20} fill={theme.colors.primary} />
              ) : now ? (
                <Loading size="sm" />
              ) : (
                <View style={{ width: 8, height: 8, borderRadius: 4, backgroundColor: theme.colors.textSecondary }} />
              )}
            </View>
            <Text style={{ fontSize: 15, color: theme.colors.text }}>{label}</Text>
          </View>
        );
      })}
    </View>
  );
}

/**
 * The assistant. `first`: the computer's first account, whose assistant begins at the welcome;
 * otherwise Back from the sign-in closes it (`close`). `enrolled` is told the new account once it
 * is set up (listed and chosen), when signing it in could not finish the assistant.
 */
export function SetupAssistant({
  host,
  first,
  accounts,
  close,
  enrolled,
  initial,
}: {
  host: AssistantHost;
  first: boolean;
  /** The enrolled accounts: the new one's name, once it is listed. */
  accounts: Accounts;
  close?: () => void;
  enrolled: (user: string, name: string) => void;
  /** Where it begins: the welcome on a first account, else the connection's check. */
  initial?: SetupStep;
}) {
  const words = useLoginMessages().setup;
  // The login screen's locale, the system's alone (ADR 0040): the frame's words speak it too.
  const locale = useShellLocale();
  const setup = host.setup;
  const [step, setStep] = useState<SetupStep>(initial ?? (first ? { step: "welcome" } : { step: "checking" }));
  // Transfer Your Data: the first account's, where the platform migrates.
  const transfer = first && !!host.migration;
  const [busy, setBusy] = useState(false);
  const [code, setCode] = useState("");
  const [appearance, setAppearance] = useState<Appearance>("auto");
  const draft = useRef<Draft>({ password: null, pin: undefined, appearance: undefined });
  // Whether the setup under way sends a PIN: its stages then include the PIN's.
  const withPin = useRef(false);
  const forget = () => {
    draft.current = { password: null, pin: undefined, appearance: undefined };
  };
  // The platform's checks as they are now: a new render does not check again.
  const latest = useRef(host);
  latest.current = host;
  // Asked once: the accounts' service's answer for this setup.
  const [platform] = useState(() => setup.platform());
  // The connection, checked before the sign-in: setup is online only.
  useEffect(() => {
    if (step.step !== "checking") return undefined;
    let current = true;
    latest.current
      .online(platform?.api)
      .catch(() => false)
      .then((online) => {
        if (!current) return;
        setStep(!online ? { step: "offline" } : platform ? { step: "signIn", platform } : { step: "notConfigured" });
      });
    return () => {
      current = false;
    };
  }, [step.step]);

  // Signed in, the login program's answer: a refusal (it should not be) leaves the account listed
  // and chosen, its field focused, as any account.
  const signingIn = step.step === "signingIn" ? step : null;
  const answer = host.answer;
  const asked = useRef<number | null>(null);
  useEffect(() => {
    if (!signingIn || !answer || answer.id === asked.current) return;
    if (answer.outcome === "accepted") {
      // Every stage done: the desktop takes the screen.
      setStep({ ...signingIn, accepted: true });
      return;
    }
    forget();
    enrolled(signingIn.user, signingIn.account.name);
  }, [answer?.id, signingIn?.user]);

  const giveUpSignIn = () => {
    forget();
    setup.end();
    setStep({ step: "checking" });
  };
  const goBack = () => {
    const to = backFrom(step, first, transfer);
    if (to === null) return;
    // Leaving the migration lets its backups go (a disk the service mounted for it).
    if (step.step === "migrate" && (typeof to === "string" || to.step !== "migrate")) host.migration?.end();
    if (to === "close") {
      forget();
      setup.end();
      close?.();
    } else if (to === "signIn") {
      giveUpSignIn();
    } else {
      // Back to the PIN chooses it anew.
      if (to.step === "pin" && step.step !== "pin") draft.current.pin = undefined;
      if (to.step === "password") draft.current.password = null;
      setStep(to);
    }
  };

  // A stage ends any wait: the service went on.
  const onStage: SetupProgress = {
    stage: (stage) =>
      setStep((now) => (now.step === "settingUp" ? { ...now, stage, waitUntil: undefined } : now)),
    waiting: (retryAfterSeconds) => {
      const until = waitFrom(retryAfterSeconds, Date.now());
      if (until === null) return;
      setStep((now) => (now.step === "settingUp" ? { ...now, waitUntil: until } : now));
    },
  };

  /** What an answer of the accounts' service leads to. */
  const settle = (account: SignedInAccount, told: EnrollAnswer) => {
    setBusy(false);
    const next = afterEnroll(account, told);
    switch (next.next) {
      case "enrolled": {
        const { pin, password } = draft.current;
        setStep({ step: "signingIn", account, user: next.user });
        latest.current.reloadAccounts?.();
        setup.end();
        // Straight to the desktop with what was just chosen: never asked a second time.
        const sent =
          pin ? host.signIn(next.user, { pin }) : password ? host.signIn(next.user, { password }) : false;
        asked.current = host.answer?.id ?? null;
        forget();
        if (!sent) enrolled(next.user, account.name);
        return;
      }
      case "secondFactor":
        setCode("");
        setStep({ step: "secondFactor", account, invalid: next.invalid });
        return;
      case "pin":
        draft.current.pin = undefined;
        setStep({ step: "pin", account, problem: "unusable" });
        return;
      case "password":
        draft.current.password = null;
        setStep({ step: "password", account, problem: next.problem, wait: next.wait });
        return;
      case "problem":
        setStep({ step: "settingUp", account, stage: null, problem: next.problem });
    }
  };

  /** The setup sent (or, after it failed with the sign-in kept, sent again). A platform that
      throws instead of answering failed: said, never a setup left waiting. */
  const enroll = (account: SignedInAccount, again = false) => {
    const { password, pin, appearance: chosen } = draft.current;
    withPin.current = !!pin;
    setBusy(true);
    setStep({ step: "settingUp", account, stage: null });
    (again
      ? setup.retry(onStage)
      : setup.enroll({ password, pin: pin ?? null, appearance: chosen ?? "auto" }, onStage)
    )
      .catch((): EnrollAnswer => ({ failed: { reason: "unanswered" } }))
      .then((told) => settle(account, told));
  };
  const sendCode = (account: SignedInAccount, typed: string) => {
    if (busy || !typed.trim()) return;
    setBusy(true);
    setCode("");
    setStep({ step: "settingUp", account, stage: "verifying" });
    setup
      .secondFactor(typed.trim(), onStage)
      .catch((): EnrollAnswer => ({ failed: { reason: "unanswered" } }))
      .then((told) => settle(account, told));
  };
  /** The next step once the PIN is chosen (or left for later): the appearance, or, when it was
      chosen already (a retry after the password or the PIN was refused), the setup at once. */
  const afterSecrets = (account: SignedInAccount) => {
    if (draft.current.appearance) enroll(account);
    else setStep({ step: "appearance", account });
  };

  const transferPage = useTransferPage({
    fromBackup: () => setStep({ step: "migrate", migrate: { step: "source", disks: null, problem: null } }),
    asNew: () => setStep({ step: "checking" }),
  });
  const migratePage = useMigratePage({
    migration: host.migration,
    step: step.step === "migrate" ? step.migrate : null,
    setStep: (migrate: MigrateStep) => setStep((now) => (now.step === "migrate" ? { step: "migrate", migrate } : now)),
    migrated: (brought) => {
      host.migration?.end();
      latest.current.migrated?.(brought);
    },
    capsLock: host.capsLock,
    announce: host.announce,
  });

  let page: AssistantPage;
  const id = pageOf(step);
  switch (step.step) {
    case "welcome":
      page = {
        id,
        title: words.welcomeTitle,
        description: words.welcomeDescription,
        icon: <PageIcon>{(color) => <RiComputerLine width={ICON} height={ICON} fill={color} />}</PageIcon>,
        content: null,
        primary: {
          label: words.getStarted,
          onPress: () => setStep(transfer ? { step: "transfer" } : { step: "checking" }),
          testID: "login-setup-start",
        },
      };
      break;
    case "transfer":
      page = transferPage;
      break;
    case "migrate":
      page = migratePage ?? { id, content: null };
      break;
    case "offline":
    case "notConfigured":
      page = {
        id,
        title: step.step === "offline" ? words.networkTitle : words.notConfiguredTitle,
        icon: (
          <PageIcon>
            {(color) =>
              step.step === "offline" ? (
                <RiWifiOffLine width={ICON} height={ICON} fill={color} />
              ) : (
                <RiErrorWarningLine width={ICON} height={ICON} fill={color} />
              )
            }
          </PageIcon>
        ),
        content: <Message text={step.step === "offline" ? words.offline : words.notConfigured} />,
        primary: { label: words.retry, onPress: () => setStep({ step: "checking" }), testID: "login-setup-retry" },
      };
      break;
    case "checking":
      page = {
        id,
        title: words.title,
        content: (
          <View style={{ alignItems: "center", gap: 12, paddingVertical: 24 }}>
            <Loading size="lg" />
            <Message text={words.checking} />
          </View>
        ),
      };
      break;
    case "signIn":
      page = { id, signIn: true, content: null };
      break;
    case "password": {
      const { account } = step;
      const said =
        step.problem === "incorrect"
          ? words.passwordIncorrect
          : step.problem === "throttled"
            ? words.passwordThrottled(step.wait ?? 0)
            : step.problem === "unusable"
              ? words.unusablePassword
              : null;
      page = {
        id,
        title: words.passwordTitle(account.name),
        description: words.passwordDescription,
        icon: <PageIcon>{(color) => <RiKey2Line width={ICON} height={ICON} fill={color} />}</PageIcon>,
        initialFocus: "content",
        content: (
          <>
            <PasswordField
              key={`password-${step.problem ?? ""}`}
              testID="login-setup"
              placeholder={words.passwordPlaceholder}
              accessibilityLabel={words.passwordPlaceholder}
              authenticate={(password) => {
                draft.current.password = password;
                if (draft.current.pin !== undefined) afterSecrets(account);
                else setStep({ step: "pin", account });
                return true;
              }}
              answer={undefined}
              capsLock={host.capsLock}
              announce={host.announce}
            />
            {said ? <Message text={said} tone="problem" /> : null}
          </>
        ),
      };
      break;
    }
    case "pin": {
      const { account } = step;
      const again = step.first !== undefined;
      const said = step.problem === "mismatch" ? words.pinMismatch : step.problem === "unusable" ? words.unusablePin : null;
      page = {
        id,
        title: again ? words.pinAgainTitle : words.pinTitle,
        description: again ? words.pinAgainDescription : pinOptional(account) ? words.pinOptionalDescription : words.pinDescription,
        icon: <PageIcon>{(color) => <RiShieldCheckLine width={ICON} height={ICON} fill={color} />}</PageIcon>,
        initialFocus: "content",
        content: (
          <>
            <PasswordField
              key={again ? "again" : `first-${step.problem ?? ""}`}
              testID={again ? "login-setup-pin-again" : "login-setup-pin"}
              passcode
              placeholder={words.pinPlaceholder}
              accessibilityLabel={again ? words.pinAgainTitle : words.pinTitle}
              authenticate={(pin) => {
                if (!again) {
                  setStep(usablePin(pin) ? { step: "pin", account, first: pin } : { step: "pin", account, problem: "unusable" });
                } else if (pin !== step.first) {
                  setStep({ step: "pin", account, problem: "mismatch" });
                } else {
                  draft.current.pin = pin;
                  afterSecrets(account);
                }
                return true;
              }}
              answer={undefined}
              announce={host.announce}
            />
            {said ? <Message text={said} tone="problem" /> : null}
          </>
        ),
        secondary:
          pinOptional(account) && !again
            ? {
                label: words.notNow,
                testID: "login-setup-pin-later",
                onPress: () => {
                  draft.current.pin = null;
                  afterSecrets(account);
                },
              }
            : undefined,
      };
      break;
    }
    case "appearance": {
      const { account } = step;
      page = {
        id,
        title: words.appearanceTitle,
        description: words.appearanceDescription,
        icon: <PageIcon>{(color) => <RiPaletteLine width={ICON} height={ICON} fill={color} />}</PageIcon>,
        content: (
          <ChoiceList
            label={words.appearanceTitle}
            value={appearance}
            onValueChange={(value) => setAppearance(value as Appearance)}
            options={APPEARANCES.map((one) => ({ value: one, label: words.appearance[one], testID: `login-setup-appearance-${one}` }))}
            style={{ flexDirection: "row", gap: 16, justifyContent: "center" }}
            optionStyle={{ padding: 6, borderRadius: 14 }}
            renderContent={(option, { selected }) => (
              <View style={{ alignItems: "center", gap: 8, opacity: selected ? 1 : 0.7 }}>
                <AppearancePreview appearance={option.value as Appearance} />
                <Text style={{ fontSize: 14, fontWeight: selected ? "600" : "400" }}>{option.label}</Text>
              </View>
            )}
          />
        ),
        primary: {
          label: words.continue,
          testID: "login-setup-continue",
          onPress: () => {
            draft.current.appearance = appearance;
            enroll(account);
          },
        },
      };
      break;
    }
    case "settingUp":
    case "signingIn":
    case "secondFactor": {
      const { account } = step;
      const problem = step.step === "settingUp" ? step.problem : undefined;
      if (step.step === "secondFactor") {
        page = {
          id,
          title: words.secondFactorTitle,
          description: words.secondFactorDescription,
          icon: <PageIcon>{(color) => <RiShieldCheckLine width={ICON} height={ICON} fill={color} />}</PageIcon>,
          initialFocus: "content",
          content: (
            <>
              <CodeField code={code} setCode={setCode} send={(typed) => sendCode(account, typed)} />
              {step.invalid ? <Message text={words.secondFactorInvalid} tone="problem" /> : null}
            </>
          ),
          primary: { label: words.continue, testID: "login-setup-continue", busy, onPress: () => sendCode(account, code) },
        };
      } else if (problem) {
        page = {
          id,
          title: words.problemTitle,
          icon: <PageIcon>{(color) => <RiErrorWarningLine width={ICON} height={ICON} fill={color} />}</PageIcon>,
          content: <Message text={words[problem]} tone="problem" />,
          primary:
            problem === "expired"
              ? { label: words.signInAgain, testID: "login-setup-sign-in-again", onPress: giveUpSignIn }
              : {
                  label: words.retry,
                  testID: "login-setup-retry",
                  // What the service kept is tried again; anything else is set up anew.
                  onPress: () => enroll(account, problem === "unreachable" || problem === "machine"),
                },
        };
      } else {
        const stage = step.step === "signingIn" ? (step.accepted ? "done" : "starting") : step.stage;
        page = {
          id,
          title:
            step.step === "signingIn"
              ? words.welcomeName(account.name || accounts?.find((one) => one.user === step.user)?.name || "")
              : words.settingUpTitle,
          description: step.step === "signingIn" ? undefined : words.settingUpDescription,
          content: (
            <View style={{ gap: 20 }}>
              <Stages stage={stage} pin={withPin.current} />
              {step.step === "settingUp" && step.waitUntil ? <Waiting key={step.waitUntil} until={step.waitUntil} /> : null}
            </View>
          ),
        };
      }
      break;
    }
  }

  const pages = pagesFor(step, first, transfer).map(
    (pageId): AssistantPage => (pageId === id ? page : { id: pageId, content: null, signIn: pageId === "signIn" ? true : undefined }),
  );
  const Frame = setup.Assistant;
  const back = backFrom(step, first, transfer) !== null && !busy ? goBack : undefined;
  return (
    <Frame
      label={words.title}
      pages={pages}
      current={id}
      onBack={back}
      backLabel={words.back}
      platform={platform}
      locale={locale}
      onSignedIn={(signedIn, password) => {
        forget();
        draft.current.password = signedIn.via === "password" ? (password ?? null) : null;
        // A password sign-in that handed over no password is set up as any other.
        const account: SignedInAccount =
          signedIn.via === "password" && !draft.current.password ? { ...signedIn, via: "other" } : signedIn;
        setStep(asksPassword(account) ? { step: "password", account } : { step: "pin", account });
      }}
    />
  );
}
