import React, { useEffect, useRef, useState, type ComponentType } from "react";
import {
  AccessibilityInfo,
  Pressable,
  ScrollView,
  TextInput,
  useWindowDimensions,
  type ImageSourcePropType,
} from "react-native";
import { ChoiceList } from "@oxy.so/bloom/choice-list";
import { Text } from "@oxy.so/bloom/typography";
import { Box } from "../../shell/src/primitives";
import { LockBackdrop } from "../../shell/src/components/LockScreen";
import { UserPicture } from "../../shell/src/components/UserPicture";
import { PasswordField, textShadow } from "../../shell/src/components/PasswordField";
import type { ShellLock } from "../../shell/src/model";
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

export type { LoginAccount, Accounts } from "./model";

/** The SDK's own sign-in, as the platform runs it for setup: it calls `onSignedIn` once the
    credential a sign-in committed was handed over, and the account has a password. */
export type SetupSignInProps = {
  platform: SetupPlatform;
  onSignedIn: (account: SignedInAccount) => void;
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
    // sign-in: its words in its own colours, legible whatever the wallpaper.
    content = (
      <Box
        className="mt-4 overflow-hidden"
        style={{ width: 512, maxWidth: "100%", backgroundColor: "rgba(18,18,22,0.88)", borderRadius: 24 }}
      >
        {/* Taller than the screen (a code under a form), it scrolls within its card. */}
        <ScrollView
          testID="login-setup-card"
          style={{ maxHeight: Math.max(240, height - 150) }}
          contentContainerStyle={{ paddingHorizontal: 32, paddingVertical: 28 }}
        >
          <SignIn platform={step.platform} onSignedIn={(account) => setStep({ step: "password", account })} />
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
      style={step.step === "signIn" ? { width: 512, maxWidth: "100%" } : { maxWidth: 360 }}
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
  wallpaper: ImageSourcePropType;
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
  } else if (shown) {
    content = (
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
    <LockBackdrop testID="login" wallpaper={wallpaper} now={now} Masked={Masked} clock={shown?.step !== "signIn"}>
      {content}
    </LockBackdrop>
  );
}
