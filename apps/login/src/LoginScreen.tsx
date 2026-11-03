import React, { useEffect, useRef, useState, type ComponentType } from "react";
import { Pressable, type ImageSourcePropType } from "react-native";
import { ChoiceList } from "@oxy.so/bloom/choice-list";
import { Text } from "@oxy.so/bloom/typography";
import { Box } from "../../shell/src/primitives";
import { LockBackdrop } from "../../shell/src/components/LockScreen";
import { UserPicture } from "../../shell/src/components/UserPicture";
import { PasswordField, textShadow } from "../../shell/src/components/PasswordField";
import type { ShellLock } from "../../shell/src/model";
import type { MaskedProps } from "../../shell/src/platform";
import {
  chosenAccount,
  opensOnSetup,
  setupAfterCheck,
  type Accounts,
  type LoginAccount,
  type Setup,
} from "./model";
import { useLoginMessages } from "./useLoginMessages";

export type { LoginAccount, Accounts } from "./model";

/** What the login screen asks of its platform. It holds no authority over who signs in. */
export type LoginActions = {
  /** Sends the password typed for `account` (its Unix user) to be checked; false if it could
      not be sent. */
  authenticate: (account: string, password: string) => boolean;
  /** The latest answer, a new `id` for each. */
  answer?: ShellLock["answer"];
  capsLock?: boolean;
  /** Whether Oxy's API can be reached now: setup is online only. */
  online: () => Promise<boolean>;
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

/** Adding an account: online only, as ADR 0020 decides; offline it says so and does nothing. */
function SetupPanel({
  setup,
  retry,
  back,
}: {
  setup: Setup;
  retry: () => void;
  back?: () => void;
}) {
  const words = useLoginMessages().setup;
  const message =
    setup === "checking" ? words.checking : setup === "offline" ? words.offline : words.unavailable;
  return (
    <Box testID="login-setup" className="items-center" style={{ maxWidth: 360 }}>
      <Text style={white(17, "600")}>{words.title}</Text>
      <Text
        testID="login-setup-message"
        accessibilityRole="alert"
        accessibilityLiveRegion="polite"
        style={{ ...white(13, "400", 0.85), marginTop: 10, textAlign: "center" }}
      >
        {message}
      </Text>
      {setup !== "checking" && <Link testID="login-setup-retry" label={words.retry} onPress={retry} />}
      {back && <Link testID="login-setup-back" label={words.back} onPress={back} />}
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
  const [setup, setSetup] = useState<Setup>("closed");
  // The first boot: no account yet, so setup is what the screen offers.
  const first = opensOnSetup(accounts);
  const shownSetup: Setup = setup === "closed" && first ? "checking" : setup;
  // The platform's check, as it is now: a new render does not check again.
  const online = useRef(actions.online);
  online.current = actions.online;
  useEffect(() => {
    if (shownSetup !== "checking") return undefined;
    let current = true;
    online
      .current()
      .catch(() => false)
      .then((online) => {
        if (current) setSetup(setupAfterCheck(online));
      });
    return () => {
      current = false;
    };
  }, [shownSetup]);
  const account = chosenAccount(accounts, chosen);
  let content: React.ReactNode;
  if (accounts === null) {
    content = (
      <Text testID="login-unavailable" accessibilityRole="alert" style={white(13, "400", 0.85)}>
        {words.unreadable}
      </Text>
    );
  } else if (shownSetup !== "closed") {
    content = (
      <SetupPanel
        setup={shownSetup}
        retry={() => setSetup("checking")}
        back={account ? () => setSetup("closed") : undefined}
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
        <Link testID="login-add" label={words.addAccount} onPress={() => setSetup("checking")} />
      </>
    );
  }
  return (
    <LockBackdrop testID="login" wallpaper={wallpaper} now={now} Masked={Masked}>
      {content}
    </LockBackdrop>
  );
}
