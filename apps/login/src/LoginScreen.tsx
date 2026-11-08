import React, { useEffect, useLayoutEffect, useRef, useState, type ComponentType } from "react";
import { AccessibilityInfo, Pressable } from "react-native";
import Svg, { Path } from "react-native-svg";
import { Button } from "@oxy.so/bloom/button";
import { ChoiceList } from "@oxy.so/bloom/choice-list";
import { Text } from "@oxy.so/bloom/typography";
import { Box } from "../../shell/src/primitives";
import { LockBackdrop } from "../../shell/src/components/LockScreen";
import { UserPicture } from "../../shell/src/components/UserPicture";
import { textShadow } from "../../shell/src/components/PasswordField";
import { SecretFields } from "../../shell/src/components/LockScreen";
import type { ShellLock, ShellWallpaper } from "../../shell/src/model";
import type { MaskedProps, ShellKeyboard } from "../../shell/src/platform";
import { focusForAccessibility } from "../../shell/src/primitives";
import { useShellMessages } from "../../shell/src/useShellMessages";
import { LOGIN_POWER_ACTIONS, loginPowerLabel, loginPowerRefusal, type LoginPowerAction } from "./power";
import { chosenAccount, opensOnSetup, type Accounts, type LoginAccount } from "./model";
import { SetupAssistant, type SetupActions } from "./SetupAssistant";
import { useLoginMessages } from "./useLoginMessages";

export type { LoginAccount, Accounts } from "./model";
export type { SetupActions, SetupAssistantFrameProps, AssistantPage, EnrollRequest, SetupProgress } from "./SetupAssistant";

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
  /** Sleep, Restart and Shut Down at the foot (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx,
      decision 8): asked of the login screen's session, which asks logind at once; its answer
      (`{refused: {reason}}` when it did not happen). Without it, no buttons. */
  power?: (action: LoginPowerAction) => Promise<unknown>;
  /** Rises at each press of the power key the session told (`{"powerKey": {}}`): the choice of
      Restart, Sleep, Cancel and Shut Down is shown. */
  powerKey?: number;
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

/** Each button's glyph, white on its round button, as a Mac's login window draws them. */
function PowerGlyph({ action }: { action: LoginPowerAction }) {
  const stroke = { stroke: "#fff", strokeWidth: 2, strokeLinecap: "round" as const, fill: "none" };
  return (
    <Svg width={22} height={22} viewBox="0 0 24 24">
      {action === "sleep" ? (
        // A crescent moon.
        <Path d="M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5Z" fill="#fff" />
      ) : action === "restart" ? (
        <>
          {/* A circle's arrow, going round to where it began. */}
          <Path d="M15.5 5.94A7 7 0 1 0 19 12" {...stroke} />
          <Path d="M16.6 12.2 19 9.6l2.4 2.6" {...stroke} strokeLinejoin="round" />
        </>
      ) : (
        <>
          {/* The power symbol. */}
          <Path d="M8 6.8a7 7 0 1 0 8 0" {...stroke} />
          <Path d="M12 3v8" {...stroke} />
        </>
      )}
    </Svg>
  );
}

/**
 * Sleep, Restart and Shut Down at the login screen's foot, centred, as a Mac's login window: round
 * buttons with their glyph and their name under it, each a button reached with Tab and named by
 * its name. Pressed, they act at once (nobody is signed in: nothing to confirm); what kept one
 * from happening is said under them.
 */
function PowerButtons({ power }: { power: (action: LoginPowerAction) => Promise<unknown> }) {
  const messages = useShellMessages();
  const [refusal, setRefusal] = useState<string | null>(null);
  const press = async (action: LoginPowerAction) => {
    setRefusal(null);
    setRefusal(loginPowerRefusal(action, await power(action), messages));
  };
  return (
    <Box className="items-center" style={{ gap: 10 }}>
      <Box className="flex-row" style={{ gap: 36 }}>
        {LOGIN_POWER_ACTIONS.map((action) => {
          const label = loginPowerLabel(action, messages);
          return (
            <Pressable
              key={action}
              testID={`login-power-${action}`}
              accessibilityRole="button"
              accessibilityLabel={label}
              onPress={() => void press(action)}
              style={{ alignItems: "center", gap: 6 }}
            >
              <Box
                className="items-center justify-center"
                style={{ width: 44, height: 44, borderRadius: 22, backgroundColor: "rgba(255,255,255,0.18)" }}
              >
                <PowerGlyph action={action} />
              </Box>
              <Text importantForAccessibility="no-hide-descendants" style={white(12, "400", 0.9)}>
                {label}
              </Text>
            </Pressable>
          );
        })}
      </Box>
      {refusal ? (
        <Text
          testID="login-power-refusal"
          accessibilityRole="alert"
          style={{ ...white(12, "400", 0.9), maxWidth: 420, textAlign: "center" }}
        >
          {refusal}
        </Text>
      ) : null}
    </Box>
  );
}

/**
 * The power key's choice at the login screen (docs/adr/0064, decision 8), as a Mac's: "Are you
 * sure you want to shut down your computer now?" with Restart and Sleep at its foot's start,
 * Cancel and Shut Down, the default: the keyboard starts on it and Return presses it; Escape
 * cancels. Modal, said as an alert as it appears. Nothing moves as it shows.
 */
function PowerKeyChoice({
  power,
  close,
  keyboard,
}: {
  power: (action: LoginPowerAction) => Promise<unknown>;
  close: () => void;
  keyboard?: ShellKeyboard;
}) {
  const messages = useShellMessages();
  const words = messages.power;
  const shutDown = useRef<React.ComponentRef<typeof Button>>(null);
  useLayoutEffect(() => {
    focusForAccessibility(shutDown.current as never);
  }, []);
  const choose = (action: LoginPowerAction) => {
    close();
    void power(action);
  };
  const latest = useRef({ choose, close });
  latest.current = { choose, close };
  useEffect(() => {
    if (!keyboard) return undefined;
    return keyboard.subscribe((event) => {
      if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return false;
      if (event.key === "Escape") latest.current.close();
      else if (event.key === "Enter") latest.current.choose("shutDown");
      else return false;
      return true;
    });
  }, [keyboard]);
  return (
    <Box
      testID="login-power-key-backdrop"
      className="absolute inset-0 items-center justify-center"
      style={{ backgroundColor: "#00000055" }}
    >
      <Box
        testID="login-power-key"
        accessibilityViewIsModal
        accessibilityRole="alert"
        accessibilityLabel={words.question.shutDown}
        style={{ width: 520, borderRadius: 18, padding: 20, gap: 18, backgroundColor: "#ffffffee" }}
      >
        <Text importantForAccessibility="no-hide-descendants" style={{ fontSize: 17, fontWeight: "600", color: "#1d2430" }}>
          {words.question.shutDown}
        </Text>
        {/* Shut Down first in the tree and last on screen (row-reverse): the platform gives a
            modal's first control the keyboard, and the default button is where it starts, as on a
            Mac; Tab then goes Cancel, Sleep, Restart. */}
        <Box style={{ flexDirection: "row-reverse", gap: 8 }}>
          <Button
            ref={shutDown}
            testID="login-power-key-shutDown"
            appearance="solid"
            tone="accent"
            onPress={() => choose("shutDown")}
          >
            {words.shutDownButton}
          </Button>
          <Button testID="login-power-key-cancel" appearance="outline" tone="neutral" onPress={close}>
            {messages.sheet.cancel}
          </Button>
          <Box style={{ flexDirection: "row-reverse", gap: 4, marginRight: "auto" }}>
            <Button testID="login-power-key-sleep" appearance="plain" tone="neutral" onPress={() => choose("sleep")}>
              {words.sleep}
            </Button>
            <Button testID="login-power-key-restart" appearance="plain" tone="neutral" onPress={() => choose("restart")}>
              {words.restartNow}
            </Button>
          </Box>
        </Box>
      </Box>
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
  safeMode = false,
  keyboard,
}: {
  accounts: Accounts;
  actions: LoginActions;
  wallpaper: ShellWallpaper;
  now: Date;
  Masked?: ComponentType<MaskedProps>;
  /** The machine started in Safe Mode (docs/adr/0054-recovery-and-erasing.mdx, decision 10): said
      at the top right, in red, as a Mac's login window says "Safe Boot". */
  safeMode?: boolean;
  /** The keys its surface receives (Escape and Return in the power key's choice). */
  keyboard?: ShellKeyboard;
}) {
  const words = useLoginMessages();
  // The power key's choice, shown at each press the session told, until answered.
  const [choosing, setChoosing] = useState(false);
  const pressed = useRef(actions.powerKey);
  useEffect(() => {
    if (actions.powerKey === pressed.current) return;
    pressed.current = actions.powerKey;
    if (actions.power) setChoosing(true);
  }, [actions.powerKey]);
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
      footer={actions.power && !shown ? <PowerButtons power={actions.power} /> : undefined}
      overlay={
        choosing && actions.power ? (
          <PowerKeyChoice power={actions.power} close={() => setChoosing(false)} keyboard={keyboard} />
        ) : undefined
      }
    >
      {content}
    </LockBackdrop>
  );
}
