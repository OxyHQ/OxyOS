import React, { useEffect, useState, type ComponentType } from "react";
import { AccessibilityInfo, Pressable } from "react-native";
import { ChoiceList } from "@oxy.so/bloom/choice-list";
import { Text } from "@oxy.so/bloom/typography";
import { Box } from "../../shell/src/primitives";
import { LockBackdrop } from "../../shell/src/components/LockScreen";
import { UserPicture } from "../../shell/src/components/UserPicture";
import { textShadow } from "../../shell/src/components/PasswordField";
import { SecretFields } from "../../shell/src/components/LockScreen";
import type { ShellLock, ShellWallpaper } from "../../shell/src/model";
import type { MaskedProps } from "../../shell/src/platform";
import { chosenAccount, opensOnSetup, type Accounts, type LoginAccount } from "./model";
import { SetupAssistant, type SetupActions } from "./SetupAssistant";
import { useLoginMessages } from "./useLoginMessages";

export type { LoginAccount, Accounts } from "./model";
export type { SetupActions, SetupAssistantFrameProps, AssistantPage, EnrollRequest } from "./SetupAssistant";

/** What the login screen asks of its platform. It holds no authority over who signs in. */
export type LoginActions = {
  /** Sends the password typed for `account` (its Unix user) to be checked; false if it could
      not be sent. */
  authenticate: (account: string, password: string) => boolean;
  /** Sends the PIN typed for `account` to be checked as its PIN for this computer; false if it
      could not be sent. */
  authenticatePin?: (account: string, pin: string) => boolean;
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
  safeMode = false,
}: {
  accounts: Accounts;
  actions: LoginActions;
  wallpaper: ShellWallpaper;
  now: Date;
  Masked?: ComponentType<MaskedProps>;
  /** The machine started in Safe Mode (docs/adr/0054-recovery-and-erasing.mdx, decision 10): said
      at the top right, in red, as a Mac's login window says "Safe Boot". */
  safeMode?: boolean;
}) {
  const words = useLoginMessages();
  const [chosen, setChosen] = useState<string | null>(null);
  // Choosing the account already chosen goes on to its password, as Return on a Mac's chosen
  // account does: its field takes the focus again.
  const [toPassword, setToPassword] = useState(0);
  // Setup: closed (null), or open, and whether it opened as the computer's first account (its
  // welcome, and no going back to an empty list). The first boot opens on it, and it stays open
  // once the account it adds is listed: the assistant ends it, signed in.
  const [settingUp, setSettingUp] = useState<{ first: boolean } | null>(null);
  const firstBoot = opensOnSetup(accounts);
  useEffect(() => {
    if (firstBoot && !settingUp) setSettingUp({ first: true });
  }, [firstBoot, settingUp]);
  const open = settingUp ?? (firstBoot ? { first: true } : null);
  const shown = open !== null;
  const first = open?.first ?? false;
  const account = chosenAccount(accounts, chosen);
  let content: React.ReactNode;
  if (accounts === null) {
    content = (
      <Text testID="login-unavailable" accessibilityRole="alert" style={white(13, "400", 0.85)}>
        {words.unreadable}
      </Text>
    );
  } else if (shown && actions.setup) {
    const setup = actions.setup;
    content = (
      <SetupAssistant
        first={first}
        accounts={accounts}
        host={{
          setup,
          online: actions.online,
          signIn: (user, secret) =>
            "pin" in secret
              ? !!actions.authenticatePin?.(user, secret.pin)
              : actions.authenticate(user, secret.password),
          answer: actions.answer,
          capsLock: actions.capsLock,
          announce: actions.announce,
          reloadAccounts: actions.reloadAccounts,
        }}
        close={account && !first ? () => setSettingUp(null) : undefined}
        enrolled={(user, name) => {
          // Set up but not signed in (the login program refused what was just chosen): the new
          // account listed and chosen, its field focused, said once.
          const added = words.setup.added(name || user);
          if (actions.announce) actions.announce(added, false);
          else AccessibilityInfo.announceForAccessibility(added);
          actions.reloadAccounts?.();
          setChosen(user);
          setSettingUp(null);
        }}
      />
    );
  } else if (shown) {
    content = (
      <Text testID="login-setup-message" accessibilityRole="alert" style={white(13, "400", 0.85)}>
        {words.setup.notConfigured}
      </Text>
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
        {/* Fields of their own for each account: what was typed for one is never sent for
            another. Its PIN first when it has one, as Windows Hello offers it; a PIN locked out
            with no password to fall back on is set anew by signing in with Oxy. */}
        {account.pin === "locked" && account.password === false ? (
          <>
            <Text
              testID="login-pin-locked"
              accessibilityRole="alert"
              style={{ ...white(13, "400", 0.9), marginTop: 12, maxWidth: 320, textAlign: "center" }}
            >
              {words.pinLocked}
            </Text>
            <Link testID="login-sign-in-oxy" label={words.signInWithOxy} onPress={() => setSettingUp({ first: false })} />
          </>
        ) : (
          <SecretFields
            key={account.user}
            testID="login"
            pin={account.pin}
            password={account.password}
            authenticate={(password) => actions.authenticate(account.user, password)}
            authenticatePin={
              actions.authenticatePin ? (pin) => actions.authenticatePin!(account.user, pin) : undefined
            }
            answer={actions.answer}
            capsLock={actions.capsLock}
            announce={actions.announce}
            focusRequest={toPassword}
          />
        )}
        <Link testID="login-add" label={words.addAccount} onPress={() => setSettingUp({ first: false })} />
      </>
    );
  }
  return (
    <LockBackdrop
      testID="login"
      wallpaper={wallpaper}
      now={now}
      Masked={Masked}
      clock={!shown}
      safeMode={safeMode}
    >
      {content}
    </LockBackdrop>
  );
}
