/**
 * Settings › Passwords (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decisions 2 to 4 and 8),
 * as a Mac's Passwords: the Oxy Keychain of the person's account. Opening it asks for the account's
 * password; then the keychain's items by category, searched, each shown in a sheet (its password
 * hidden until shown, copied marked sensitive and cleared after 90 seconds, its verification code
 * counting down), edited, made and deleted; the warnings; the devices that hold it and those
 * waiting to. Turning it on shows the recovery key once and asks for it back; joining it is another
 * device's approval or the recovery key.
 *
 * The accounts' service holds the vault and decides everything: this page asks it item by item and
 * shows what it is told. It holds no key; a password, a code or a recovery key lives here only
 * while it is shown, and the strong password is the service's generator's.
 */
import * as React from "react";
import { AccessibilityInfo, TextInput, View } from "react-native";
import { AlertDialog } from "@oxy.so/bloom/alert-dialog";
import { Button } from "@oxy.so/bloom/button";
import { CategoryBar, type CategoryBarItem } from "@oxy.so/bloom/category-bar";
import { Dialog, type DialogAction } from "@oxy.so/bloom/dialog";
import { GroupedSection, GroupedSectionItem } from "@oxy.so/bloom/grouped-section";
import {
  RiAddLine,
  RiComputerLine,
  RiErrorWarningLine,
  RiFileCopyLine,
  RiKey2Line,
  RiLayoutGridLine,
  RiLockLine,
  RiShieldCheckLine,
  RiShieldUserLine,
  RiStickyNoteLine,
  RiTimerLine,
  RiWifiLine,
} from "@oxy.so/bloom/icons";
import { Loading } from "@oxy.so/bloom/loading";
import { Search } from "@oxy.so/bloom/search";
import { TextField, TextFieldHint, TextFieldInput } from "@oxy.so/bloom/text-field";
import { useTheme } from "@oxy.so/bloom/theme";
import { Text } from "@oxy.so/bloom/typography";
import type { Messages } from "./messages";
import type { SettingsActions, SettingsSnapshot } from "./model";
import {
  ACCESS_IDLE_MS,
  CATEGORIES,
  closesAccess,
  formatCode,
  initialOf,
  itemsIn,
  keychainClient,
  otpauthFrom,
  outcomeMessage,
  personsRequest,
  recoveryKeyGroups,
  search,
  stageOf,
  staysOpen,
  urlsFromText,
  warningsOf,
  type Audit,
  type Category,
  type Enrollment,
  type ItemInput,
  type ItemSummary,
  type ItemView,
  type KeychainClient,
  type KeychainStatus,
  type Member,
  type Outcome,
} from "./passwords";
import type { PasswordsMessages } from "./messages";

type Props = { snapshot: SettingsSnapshot; actions: SettingsActions; strings: Messages; locale: string };
type Words = PasswordsMessages;
type Refused = Exclude<Outcome<unknown>, { ok: unknown }>;

const say = (text: string) => AccessibilityInfo.announceForAccessibility(text);
/** A request that could not be made at all, as an outcome. */
const unreachable = { failed: "unavailable" } as const;
async function asked<T>(request: Promise<Outcome<T>>): Promise<Outcome<T>> {
  return request.catch(() => unreachable);
}

export function PasswordsPage({ actions, strings, locale }: Props) {
  const words = strings.passwords;
  const platform = actions.keychain;
  // Five minutes with nothing asked of the keychain by the person (decision 8), whatever the page
  // shows (the open keychain, a recovery key, a join's code): it is made anew and the password
  // asked again. What the page asks on its own (a status, a code as it runs out, a join or an
  // approval followed, the devices listed again) is no activity.
  const lapsed = React.useRef<() => void>(() => {});
  const idle = React.useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  React.useEffect(() => () => idle.current && clearTimeout(idle.current), []);
  const client = React.useMemo(() => {
    if (!platform) return null;
    return keychainClient({
      ...platform,
      ask: (request) => {
        if (personsRequest(request)) {
          if (idle.current) clearTimeout(idle.current);
          idle.current = setTimeout(() => lapsed.current(), ACCESS_IDLE_MS);
        }
        return platform.ask(request);
      },
    });
  }, [platform]);
  // undefined while asked; null when the service could not say (no account session, refused).
  const [status, setStatus] = React.useState<KeychainStatus | null | undefined>(undefined);
  const turn = React.useRef(0);
  const refresh = React.useCallback(async () => {
    if (!client) return setStatus(null);
    const mine = ++turn.current;
    const told = await asked(client.status());
    if (mine === turn.current) setStatus("ok" in told ? told.ok : null);
  }, [client]);
  React.useEffect(() => {
    void refresh();
  }, [refresh]);
  // Closed here (five minutes idle, the proof lapsed): the password again, whatever the service
  // still says, until it is given.
  const [closed, setClosed] = React.useState(false);
  // Each making of the page's stages anew (closed): what they held is forgotten.
  const [generation, setGeneration] = React.useState(0);
  const close = React.useCallback(async () => {
    setClosed(true);
    await refresh();
  }, [refresh]);
  const verified = React.useCallback(async () => {
    setClosed(false);
    await refresh();
  }, [refresh]);
  lapsed.current = () => {
    setClosed(true);
    setGeneration((count) => count + 1);
    void refresh();
  };
  // The session's lock leaves no window active (docs/adr/0016-windows.mdx) and locks the keychain,
  // but tells applications nothing else. So whenever Settings' window is not the active one (it
  // stopped being, or another application was active when the session locked), the service is
  // asked every few seconds whether the keychain is still unlocked and the person verified
  // (`keychainStatus`, which renews nothing); when the window becomes active again nothing of the
  // page shows, its sheets included, until the service said so. Locked, or the proof gone with the
  // lock (a PIN never opens the keychain): every stage is made anew, forgetting what it held (a
  // list, an item, a recovery key, a code, what was typed), and the account's password is asked
  // again. While the window stays active nothing is asked: the lock makes it inactive first.
  const [hidden, setHidden] = React.useState(false);
  React.useEffect(() => {
    if (!client?.watchActive) return;
    let current = true;
    let polling: ReturnType<typeof setInterval> | undefined;
    // Whether the last answer let the page stay: it is made anew once as that ends, not at every ask.
    let stayed = true;
    const check = async () => {
      const told = await asked(client.status());
      if (!current) return;
      const now = "ok" in told ? told.ok : null;
      const stays = staysOpen(now);
      if (!stays && stayed) {
        setClosed(true);
        setGeneration((count) => count + 1);
        setStatus(now);
      }
      stayed = stays;
      setHidden(false);
    };
    const unsubscribe = client.watchActive((active) => {
      if (polling) clearInterval(polling);
      polling = undefined;
      if (active) setHidden(true);
      else polling = setInterval(() => void check(), INACTIVE_CHECK_MS);
      void check();
    });
    return () => {
      current = false;
      if (polling) clearInterval(polling);
      unsubscribe();
    };
  }, [client]);

  if (!client || status === null) return <Note text={words.unavailable} testID="passwords-unavailable" />;
  if (status === undefined) {
    return (
      <View style={{ alignItems: "center", paddingVertical: 32 }}>
        <Loading size="sm" accessibilityLabel={words.title} />
      </View>
    );
  }
  const common = { client, words, onChanged: refresh };
  const stage = stageOf(status);
  const gated = closed && !["unavailable", "locked", "needsPreviousPassword"].includes(stage);
  const content = (() => {
    switch (gated ? "gate" : stage) {
      case "unavailable":
        return <Note text={words.unavailable} testID="passwords-unavailable" />;
      case "locked":
        return <Note text={words.locked} testID="passwords-locked" />;
      case "needsPreviousPassword":
        return <PreviousPassword {...common} />;
      case "gate":
        return <Gate {...common} onChanged={verified} />;
      case "turnOn":
        return <TurnOn {...common} interrupted={false} />;
      case "creating":
        return <TurnOn {...common} interrupted />;
      case "join":
        return <Join {...common} note={null} />;
      case "removed":
        return <Join {...common} note={words.removed} />;
      case "joining":
        return <Joining {...common} />;
      case "unbound":
        return <Note text={words.unbound} testID="passwords-unbound" />;
      case "open":
        return <Browser {...common} onChanged={close} locale={locale} />;
    }
  })();
  return (
    <Hidden.Provider value={hidden}>
      <View key={generation} style={hidden ? { display: "none" } : undefined}>
        {content}
      </View>
      {hidden ? (
        <View style={{ alignItems: "center", paddingVertical: 32 }} testID="passwords-checking">
          <Loading size="sm" accessibilityLabel={words.title} />
        </View>
      ) : null}
    </Hidden.Provider>
  );
}

/** Whether the page is hidden while the service is asked whether the keychain may stay open: its
    sheets are hidden with it (they are shown above the window, not inside the page). */
const Hidden = React.createContext(false);

/** A line of the page's own, said as it shows. */
function Note({ text, testID }: { text: string; testID: string }) {
  return (
    <GroupedSection>
      <GroupedSectionItem title={text} titleNumberOfLines={0} testID={testID} />
    </GroupedSection>
  );
}

/** An error under a form, said at once (an alert). */
function ErrorLine({ text, testID }: { text: string | null; testID: string }) {
  if (!text) return null;
  return (
    <View accessibilityRole="alert" testID={testID}>
      <TextFieldHint invalid>{text}</TextFieldHint>
    </View>
  );
}

type FieldProps = {
  label: string;
  value: string;
  onChange: (text: string) => void;
  onSubmit?: () => void;
  secure?: boolean;
  autoFocus?: boolean;
  invalid?: boolean;
  editable?: boolean;
  multiline?: boolean;
  monospace?: boolean;
  testID: string;
  inputRef?: React.RefObject<React.ComponentRef<typeof TextInput> | null>;
};

/** A labelled field, as every form here has them: never corrected, never capitalized. */
function Field(props: FieldProps) {
  return (
    <TextField invalid={props.invalid}>
      <TextFieldInput
        inputRef={props.inputRef}
        label={props.label}
        placeholder={props.label}
        value={props.value}
        onChangeText={props.onChange}
        secureTextEntry={props.secure}
        autoFocus={props.autoFocus}
        autoCapitalize="none"
        autoCorrect={false}
        editable={props.editable ?? true}
        multiline={props.multiline}
        submitBehavior={props.multiline ? undefined : "submit"}
        onSubmitEditing={props.multiline ? undefined : props.onSubmit}
        style={props.monospace ? { fontFamily: "monospace" } : undefined}
        testID={props.testID}
      />
    </TextField>
  );
}

/** A page-level panel: a symbol, a heading and what it says, then its content. */
function Panel({
  icon,
  title,
  description,
  children,
  testID,
}: {
  icon: React.ReactNode;
  title: string;
  description: string;
  children?: React.ReactNode;
  testID: string;
}) {
  const theme = useTheme();
  return (
    <View style={{ alignItems: "center", gap: 12, paddingVertical: 24 }} testID={testID}>
      <View importantForAccessibility="no-hide-descendants" accessibilityElementsHidden>
        {icon}
      </View>
      <Text variant="title-3-semibold" accessibilityRole="header" style={{ textAlign: "center" }}>
        {title}
      </Text>
      <Text variant="body-regular" style={{ textAlign: "center", maxWidth: 420, color: theme.colors.textSecondary }}>
        {description}
      </Text>
      <View style={{ width: "100%", maxWidth: 360, gap: 8 }}>{children}</View>
    </View>
  );
}

function PanelIcon({ Icon }: { Icon: typeof RiLockLine }) {
  const theme = useTheme();
  return <Icon width={40} height={40} fill={theme.colors.textSecondary} />;
}

type Common = { client: KeychainClient; words: Words; onChanged: () => Promise<void> };

/** The gate (decision 8): the account's password, as the Passwords app asks for Touch ID. */
function Gate({ client, words, onChanged }: Common) {
  const [password, setPassword] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);
  const [working, setWorking] = React.useState(false);
  const field = React.useRef<React.ComponentRef<typeof TextInput>>(null);
  const submit = async () => {
    if (working || password === "") return;
    setWorking(true);
    setError(null);
    const typed = password;
    setPassword("");
    const outcome = await asked(client.verify(typed));
    setWorking(false);
    if ("ok" in outcome) return void (await onChanged());
    const said = outcomeMessage(outcome, words);
    setError(said);
    say(said);
    field.current?.focus();
  };
  return (
    <Panel
      icon={<PanelIcon Icon={RiLockLine} />}
      title={words.gateTitle}
      description={words.gateDescription}
      testID="passwords-gate"
    >
      <Field
        inputRef={field}
        label={words.password}
        value={password}
        onChange={(text) => {
          setPassword(text);
          setError(null);
        }}
        onSubmit={() => void submit()}
        secure
        autoFocus
        invalid={error !== null}
        editable={!working}
        testID="passwords-gate-password"
      />
      <ErrorLine text={error} testID="passwords-gate-error" />
      <Button
        onPress={() => void submit()}
        disabled={password === ""}
        loading={working}
        testID="passwords-gate-unlock"
      >
        {words.unlock}
      </Button>
    </Panel>
  );
}

/** A password changed elsewhere (decision 2): the previous one re-seals the keybag, or the machine
    leaves the vault. Both passwords prove the person, so no gate comes first. */
function PreviousPassword({ client, words, onChanged }: Common) {
  const hidden = React.useContext(Hidden);
  const [previous, setPrevious] = React.useState("");
  const [current, setCurrent] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);
  const [working, setWorking] = React.useState(false);
  const [leaving, setLeaving] = React.useState(false);
  const submit = async () => {
    if (working || previous === "" || current === "") return;
    setWorking(true);
    setError(null);
    const outcome = await asked(client.rekey(previous, current));
    setPrevious("");
    setCurrent("");
    setWorking(false);
    if ("ok" in outcome) return void (await onChanged());
    const said = outcomeMessage(outcome, words);
    setError(said);
    say(said);
  };
  const leave = async () => {
    const outcome = await asked(client.leave());
    if (!("ok" in outcome)) {
      const said = outcomeMessage(outcome, words);
      setError(said);
      say(said);
    }
    await onChanged();
  };
  return (
    <Panel
      icon={<PanelIcon Icon={RiKey2Line} />}
      title={words.previousTitle}
      description={words.previousDescription}
      testID="passwords-previous"
    >
      <Field
        label={words.previousPassword}
        value={previous}
        onChange={setPrevious}
        secure
        autoFocus
        invalid={error !== null}
        editable={!working}
        testID="passwords-previous-password"
      />
      <Field
        label={words.currentPassword}
        value={current}
        onChange={setCurrent}
        onSubmit={() => void submit()}
        secure
        editable={!working}
        testID="passwords-current-password"
      />
      <ErrorLine text={error} testID="passwords-previous-error" />
      <Button
        onPress={() => void submit()}
        disabled={previous === "" || current === ""}
        loading={working}
        testID="passwords-previous-update"
      >
        {words.update}
      </Button>
      <Button appearance="plain" tone="neutral" onPress={() => setLeaving(true)} testID="passwords-leave">
        {words.leave}
      </Button>
      <AlertDialog
        open={leaving && !hidden}
        onOpenChange={(open) => !open && setLeaving(false)}
        title={words.leaveTitle}
        description={words.leaveDescription}
        confirmLabel={words.leaveConfirm}
        cancelLabel={words.cancel}
        destructive
        onConfirm={() => void leave()}
        testID="passwords-leave-confirm"
      />
    </Panel>
  );
}

/**
 * Turning the keychain on (decision 3): the vault made, its recovery key shown once in eight groups
 * of four, then typed back before the vault exists. A making interrupted (the sheet closed, the page
 * left) starts over with a new key: the one shown is gone.
 */
function TurnOn({ client, words, onChanged, interrupted }: Common & { interrupted: boolean }) {
  const hidden = React.useContext(Hidden);
  // The recovery key, while its sheet is open: shown, then asked back. Forgotten as it closes.
  const [key, setKey] = React.useState<string | null>(null);
  const [step, setStep] = React.useState<"show" | "confirm">("show");
  const [typed, setTyped] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);
  const [working, setWorking] = React.useState(false);
  const [opened, setOpened] = React.useState(0);
  const start = async () => {
    if (working) return;
    setWorking(true);
    setError(null);
    const outcome = await asked(client.create());
    setWorking(false);
    if ("ok" in outcome) {
      setOpened((count) => count + 1);
      setStep("show");
      setTyped("");
      setKey(outcome.ok);
      return;
    }
    if ("refused" in outcome && outcome.refused === "vaultExists") return void (await onChanged());
    const said = outcomeMessage(outcome, words);
    setError(said);
    say(said);
  };
  const close = () => {
    if (working) return;
    setKey(null);
    setTyped("");
    setError(null);
    void onChanged();
  };
  const confirm = async () => {
    if (working || typed.trim() === "") return;
    setWorking(true);
    setError(null);
    const outcome = await asked(client.confirmRecoveryKey(typed));
    setWorking(false);
    if ("ok" in outcome) {
      setKey(null);
      setTyped("");
      say(words.turnedOn);
      return void (await onChanged());
    }
    const said = outcomeMessage(outcome, words);
    setError(said);
    say(said);
  };
  const groups = key ? recoveryKeyGroups(key) : [];
  const theme = useTheme();
  const sheet =
    step === "show"
      ? {
          title: words.recoveryTitle,
          description: words.recoveryDescription,
          actions: [
            { label: words.cancel, color: "cancel", onPress: close, testID: "passwords-recovery-cancel" },
            {
              label: words.continue,
              color: "default",
              shouldCloseOnPress: false,
              initialFocus: true,
              onPress: () => {
                setStep("confirm");
                setError(null);
              },
              testID: "passwords-recovery-continue",
            },
          ] as DialogAction[],
          content: (
            <View style={{ gap: 12, paddingBottom: 12 }}>
              <View
                accessible
                accessibilityLabel={words.recoveryKeyLabel(groups.map((group) => group.split("").join(" ")).join(", "))}
                style={{
                  flexDirection: "row",
                  flexWrap: "wrap",
                  justifyContent: "center",
                  gap: 8,
                  padding: 12,
                  borderRadius: 10,
                  backgroundColor: theme.colors.borderLight,
                }}
                testID="passwords-recovery-key"
              >
                {groups.map((group, index) => (
                  <Text key={index} variant="body-regular" style={{ fontFamily: "monospace", letterSpacing: 1 }}>
                    {group}
                  </Text>
                ))}
              </View>
              <View accessibilityRole="alert" style={{ flexDirection: "row", gap: 8 }}>
                <RiErrorWarningLine width={18} height={18} fill={theme.colors.textSecondary} />
                <Text variant="body-2-regular" style={{ flex: 1 }} testID="passwords-recovery-warning">
                  {words.recoveryWarning}
                </Text>
              </View>
            </View>
          ),
        }
      : {
          title: words.confirmTitle,
          description: words.confirmDescription,
          actions: [
            { label: words.cancel, color: "cancel", onPress: close, disabled: working, testID: "passwords-recovery-cancel" },
            {
              label: words.confirm,
              color: "default",
              shouldCloseOnPress: false,
              loading: working,
              disabled: typed.trim() === "",
              onPress: () => void confirm(),
              testID: "passwords-recovery-confirm",
            },
          ] as DialogAction[],
          content: (
            <View style={{ gap: 8, paddingBottom: 12 }}>
              <Field
                label={words.recoveryKey}
                value={typed}
                onChange={(text) => {
                  setTyped(text);
                  setError(null);
                }}
                onSubmit={() => void confirm()}
                autoFocus
                monospace
                secure
                invalid={error !== null}
                editable={!working}
                testID="passwords-recovery-typed"
              />
              <ErrorLine text={error} testID="passwords-recovery-error" />
              {error ? (
                <Button
                  appearance="plain"
                  tone="neutral"
                  onPress={() => void start()}
                  testID="passwords-recovery-start-over"
                >
                  {words.startOver}
                </Button>
              ) : null}
            </View>
          ),
        };
  return (
    <Panel
      icon={<PanelIcon Icon={RiKey2Line} />}
      title={words.turnOnTitle}
      description={interrupted ? words.creatingInterrupted : words.turnOnDescription}
      testID="passwords-turn-on"
    >
      <Text variant="body-2-regular" style={{ textAlign: "center" }} testID="passwords-turn-on-warning">
        {words.recoveryWarning}
      </Text>
      {key === null ? <ErrorLine text={error} testID="passwords-turn-on-error" /> : null}
      <Button onPress={() => void start()} loading={working && key === null} testID="passwords-turn-on-button">
        {interrupted ? words.startOver : words.turnOn}
      </Button>
      <Dialog
        key={opened}
        open={key !== null && !hidden}
        onClose={close}
        title={sheet.title}
        description={sheet.description}
        label={sheet.title}
        actions={sheet.actions}
        dismissOnBackdrop={false}
        testID="passwords-recovery-sheet"
      >
        {sheet.content}
      </Dialog>
    </Panel>
  );
}

/** Joining a vault the account has (decision 4): another device's approval, or the recovery key. */
function Join({ client, words, onChanged, note }: Common & { note: string | null }) {
  const hidden = React.useContext(Hidden);
  const [asking, setAsking] = React.useState(false);
  const [typed, setTyped] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);
  const [working, setWorking] = React.useState(false);
  const byApproval = async () => {
    if (working) return;
    setWorking(true);
    setError(null);
    const outcome = await asked(client.joinByApproval());
    setWorking(false);
    if (!("ok" in outcome)) {
      const said = outcomeMessage(outcome, words);
      setError(said);
      say(said);
    }
    await onChanged();
  };
  const withKey = async () => {
    if (working || typed.trim() === "") return;
    setWorking(true);
    setError(null);
    const outcome = await asked(client.joinWithRecoveryKey(typed));
    setWorking(false);
    if ("ok" in outcome) {
      setAsking(false);
      setTyped("");
      say(words.joined);
      return void (await onChanged());
    }
    const said = outcomeMessage(outcome, words);
    setError(said);
    say(said);
  };
  const close = () => {
    if (working) return;
    setAsking(false);
    setTyped("");
    setError(null);
  };
  return (
    <Panel
      icon={<PanelIcon Icon={RiKey2Line} />}
      title={words.joinTitle}
      description={note ?? words.joinDescription}
      testID="passwords-join"
    >
      <GroupedSection>
        <GroupedSectionItem
          title={words.joinByApproval}
          description={words.joinByApprovalHint}
          descriptionLines={0}
          onPress={() => void byApproval()}
          accessibilityRole="button"
          testID="passwords-join-approval"
        />
        <GroupedSectionItem
          title={words.joinWithRecoveryKey}
          description={words.joinWithRecoveryKeyHint}
          descriptionLines={0}
          onPress={() => {
            setError(null);
            setAsking(true);
          }}
          accessibilityRole="button"
          testID="passwords-join-recovery"
        />
      </GroupedSection>
      {!asking ? <ErrorLine text={error} testID="passwords-join-error" /> : null}
      <Dialog
        open={asking && !hidden}
        onClose={close}
        title={words.joinWithRecoveryKey}
        description={words.joinWithRecoveryKeyHint}
        label={words.joinWithRecoveryKey}
        actions={[
          { label: words.cancel, color: "cancel", onPress: close, disabled: working, testID: "passwords-join-cancel" },
          {
            label: words.join,
            color: "default",
            shouldCloseOnPress: false,
            loading: working,
            disabled: typed.trim() === "",
            onPress: () => void withKey(),
            testID: "passwords-join-confirm",
          },
        ]}
        testID="passwords-join-sheet"
      >
        <View style={{ gap: 8, paddingBottom: 12 }}>
          <Field
            label={words.recoveryKey}
            value={typed}
            onChange={(text) => {
              setTyped(text);
              setError(null);
            }}
            onSubmit={() => void withKey()}
            autoFocus
            monospace
            secure
            invalid={error !== null}
            editable={!working}
            testID="passwords-join-key"
          />
          <ErrorLine text={error} testID="passwords-join-key-error" />
        </View>
      </Dialog>
    </Panel>
  );
}

/** How often Passwords asks whether the keychain is still open while Settings' window is not the
    active one (the session may lock meanwhile). */
const INACTIVE_CHECK_MS = 2000;

/** How often a join or an approval asks how it goes. */
const FOLLOW_MS = 2000;

/** Joining by approval, under way: the code both devices show, until the other approves. */
function Joining({ client, words, onChanged }: Common) {
  const [code, setCode] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  React.useEffect(() => {
    let current = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const follow = async () => {
      const outcome = await asked(client.joinByApproval());
      if (!current) return;
      if ("ok" in outcome) {
        if (outcome.ok.status === "approved") {
          say(words.joined);
          return void onChanged();
        }
        setCode((shown) => {
          const next = outcome.ok.code ?? null;
          if (next && next !== shown) say(words.joiningCode(next.split("").join(" ")));
          return next;
        });
        setError(null);
        timer = setTimeout(() => void follow(), FOLLOW_MS);
        return;
      }
      const said = outcomeMessage(outcome, words);
      setError(said);
      say(said);
      // Denied or expired: the machine is no longer joining; anything else is asked again.
      if ("refused" in outcome && (outcome.refused === "denied" || outcome.refused === "expired")) return;
      timer = setTimeout(() => void follow(), FOLLOW_MS);
    };
    void follow();
    return () => {
      current = false;
      if (timer) clearTimeout(timer);
    };
  }, [client, words, onChanged]);
  return (
    <Panel
      icon={<PanelIcon Icon={RiComputerLine} />}
      title={words.joiningTitle}
      description={words.joiningDescription}
      testID="passwords-joining"
    >
      {code ? (
        <Text
          variant="title-3-semibold"
          style={{ textAlign: "center", fontFamily: "monospace", letterSpacing: 4 }}
          accessibilityLabel={words.joiningCode(code.split("").join(" "))}
          testID="passwords-joining-code"
        >
          {formatCode(code)}
        </Text>
      ) : (
        <View style={{ flexDirection: "row", gap: 8, justifyContent: "center", alignItems: "center" }}>
          <Loading size="sm" accessibilityLabel={words.joiningWaiting} />
          <Text variant="body-regular">{words.joiningWaiting}</Text>
        </View>
      )}
      <ErrorLine text={error} testID="passwords-joining-error" />
      {error ? (
        <Button appearance="plain" tone="neutral" onPress={() => void onChanged()} testID="passwords-joining-again">
          {words.tryAgain}
        </Button>
      ) : null}
    </Panel>
  );
}

const CATEGORY_ICONS: Record<Category, typeof RiLockLine> = {
  all: RiLayoutGridLine,
  passwords: RiKey2Line,
  passkeys: RiShieldUserLine,
  codes: RiTimerLine,
  wifi: RiWifiLine,
  notes: RiStickyNoteLine,
  security: RiShieldCheckLine,
  devices: RiComputerLine,
};

/** What a sheet over the list holds: an item shown, an item being edited, or a new one. */
type Sheet =
  | { kind: "item"; itemId: string; item: ItemView | null; editing: boolean }
  | { kind: "new"; type: "login" | "note" };

/**
 * The keychain, open (decision 8). It stays open while the person does something in it: five
 * minutes with nothing asked of it (the service's own measure too), a refusal saying the proof
 * lapsed, or the session's lock, and it closes, forgetting everything shown.
 */
function Browser({ client, words, onChanged, locale }: Common & { locale: string }) {
  const [category, setCategory] = React.useState<Category>("all");
  const [query, setQuery] = React.useState("");
  const [items, setItems] = React.useState<ItemSummary[] | null>(null);
  const [audit, setAudit] = React.useState<Audit | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [sheet, setSheet] = React.useState<Sheet | null>(null);
  const [opened, setOpened] = React.useState(0);
  const idle = React.useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const lapse = React.useCallback(() => {
    if (idle.current) clearTimeout(idle.current);
    setSheet(null);
    setItems(null);
    void onChanged();
  }, [onChanged]);
  /** Something was done: five more minutes. */
  const touch = React.useCallback(() => {
    if (idle.current) clearTimeout(idle.current);
    idle.current = setTimeout(lapse, ACCESS_IDLE_MS);
  }, [lapse]);
  React.useEffect(() => () => idle.current && clearTimeout(idle.current), []);
  /** A refusal that ends the access closes the page; any other is said. True if it closed. */
  const settle = React.useCallback(
    (outcome: Refused) => {
      if (closesAccess(outcome)) {
        lapse();
        return true;
      }
      return false;
    },
    [lapse],
  );
  const load = React.useCallback(async () => {
    touch();
    const [listed, audited] = await Promise.all([asked(client.list()), asked(client.audit())]);
    if (!("ok" in listed)) {
      if (settle(listed)) return;
      setError(outcomeMessage(listed, words));
      return;
    }
    setError(null);
    setItems(listed.ok);
    if ("ok" in audited) setAudit(audited.ok);
    else if (settle(audited)) return;
  }, [client, touch, settle, words]);
  React.useEffect(() => {
    void load();
  }, [load]);
  const open = (next: Sheet) => {
    touch();
    setOpened((count) => count + 1);
    setSheet(next);
  };
  const choose = (key: string) => {
    touch();
    setCategory(key as Category);
  };
  const categoryItems: CategoryBarItem[] = CATEGORIES.map((key) => ({
    key,
    label: words.categories[key],
    icon: CATEGORY_ICONS[key],
  }));
  const listed = items ? search(itemsIn(category, items, audit), query) : [];
  const listing = category !== "devices";
  return (
    <View style={{ gap: 16 }} testID="passwords-open">
      <CategoryBar
        items={categoryItems}
        value={category}
        onValueChange={choose}
        accessibilityLabel={words.categoriesLabel}
        gap={20}
        testID="passwords-categories"
      />
      {listing ? (
        <View style={{ flexDirection: "row", gap: 8, alignItems: "center" }}>
          <View style={{ flex: 1 }}>
            <Search
              value={query}
              onChangeText={(text: string) => {
                touch();
                setQuery(text);
              }}
              onClearText={() => setQuery("")}
              placeholder={words.search}
              accessibilityLabel={words.search}
              autoCorrect={false}
              autoCapitalize="none"
              testID="passwords-search"
            />
          </View>
          <Button
            size="sm"
            appearance="outline"
            tone="neutral"
            icon={RiAddLine}
            onPress={() => open({ kind: "new", type: category === "notes" ? "note" : "login" })}
            testID="passwords-new"
          >
            {category === "notes" ? words.newNote : words.newPassword}
          </Button>
        </View>
      ) : null}
      {error ? <ErrorLine text={error} testID="passwords-error" /> : null}
      {category === "security" ? (
        <Text variant="body-2-regular" testID="passwords-security-description">
          {words.securityDescription}
        </Text>
      ) : null}
      {category === "devices" ? (
        <Devices client={client} words={words} touch={touch} settle={settle} />
      ) : items === null ? (
        error ? null : (
          <View style={{ alignItems: "center", paddingVertical: 24 }}>
            <Loading size="sm" accessibilityLabel={words.title} />
          </View>
        )
      ) : (
        <GroupedSection>
          {listed.length === 0 ? (
            <GroupedSectionItem
              title={
                query.trim() !== ""
                  ? words.noResults
                  : category === "security"
                    ? words.noWarnings
                    : words.noItems
              }
              testID="passwords-empty"
            />
          ) : (
            listed.map((item) => (
              <ItemRow
                key={item.itemId}
                item={item}
                words={words}
                audit={audit}
                warn={category === "security"}
                onPress={() => open({ kind: "item", itemId: item.itemId, item: null, editing: false })}
              />
            ))
          )}
        </GroupedSection>
      )}
      <ItemSheet
        key={opened}
        sheet={sheet}
        client={client}
        words={words}
        locale={locale}
        audit={audit}
        touch={touch}
        settle={settle}
        onSheet={setSheet}
        onSaved={() => void load()}
      />
    </View>
  );
}

/** An item's picture: its title's first letter, or its kind's symbol. */
function ItemIcon({ item }: { item: Pick<ItemSummary, "type" | "title" | "domains"> }) {
  const theme = useTheme();
  const Icon =
    item.type === "passkey"
      ? RiShieldUserLine
      : item.type === "wifi"
        ? RiWifiLine
        : item.type === "note"
          ? RiStickyNoteLine
          : item.type === "totp"
            ? RiTimerLine
            : null;
  return (
    <View
      style={{
        width: 28,
        height: 28,
        borderRadius: 7,
        alignItems: "center",
        justifyContent: "center",
        backgroundColor: theme.colors.borderLight,
      }}
      importantForAccessibility="no-hide-descendants"
      accessibilityElementsHidden
    >
      {Icon ? (
        <Icon width={16} height={16} fill={theme.colors.textSecondary} />
      ) : (
        <Text variant="body-2-regular" style={{ fontWeight: "600" }}>
          {initialOf(item)}
        </Text>
      )}
    </View>
  );
}

/** What a row says under its title. */
function rowDetail(item: ItemSummary, words: Words, audit: Audit | null, warn: boolean): string {
  if (warn) {
    const { reusedWith, weak } = warningsOf(item.itemId, audit);
    if (weak && reusedWith > 0) return words.reusedAndWeak(reusedWith);
    if (weak) return words.weak;
    if (reusedWith > 0) return words.reused(reusedWith);
  }
  if (item.type === "passkey") return words.passkeyFor(item.domains[0] ?? item.title);
  if (item.type === "note" || item.type === "wifi") return item.type === "wifi" ? words.categories.wifi : words.categories.notes;
  return item.username || words.noUsername;
}

function ItemRow({
  item,
  words,
  audit,
  warn,
  onPress,
}: {
  item: ItemSummary;
  words: Words;
  audit: Audit | null;
  warn: boolean;
  onPress: () => void;
}) {
  const detail = rowDetail(item, words, audit, warn);
  return (
    <GroupedSectionItem
      icon={<ItemIcon item={item} />}
      title={item.title}
      description={detail}
      onPress={onPress}
      accessibilityLabel={`${item.title}, ${detail}`}
      testID={`passwords-item-${item.itemId}`}
    />
  );
}

/** A value shown in a sheet's row, with its buttons. */
function ValueRow({
  title,
  value,
  testID,
  accessibilityLabel,
  monospace,
  children,
}: {
  title: string;
  value: string;
  testID: string;
  accessibilityLabel?: string;
  monospace?: boolean;
  children?: React.ReactNode;
}) {
  const theme = useTheme();
  return (
    <View style={{ flexDirection: "row", alignItems: "center", gap: 8, paddingVertical: 6 }}>
      <View style={{ flex: 1 }} accessible accessibilityLabel={accessibilityLabel ?? `${title}, ${value}`} testID={testID}>
        <Text variant="caption-1-regular" style={{ color: theme.colors.textSecondary }}>
          {title}
        </Text>
        <Text variant="body-regular" selectable={false} style={monospace ? { fontFamily: "monospace" } : undefined}>
          {value}
        </Text>
      </View>
      {children}
    </View>
  );
}

function SmallButton({
  label,
  onPress,
  testID,
  accessibilityLabel,
  icon,
}: {
  label: string;
  onPress: () => void;
  testID: string;
  accessibilityLabel?: string;
  icon?: typeof RiLockLine;
}) {
  return (
    <Button
      size="sm"
      appearance="outline"
      tone="neutral"
      icon={icon}
      onPress={onPress}
      accessibilityLabel={accessibilityLabel ?? label}
      testID={testID}
    >
      {label}
    </Button>
  );
}

/** An item's verification code now, counting down, asked again as each runs out (`keychainCode`:
    asking it renews nothing, so a code left on screen never keeps the keychain open). */
function LiveCode({
  client,
  itemId,
  words,
  settle,
  onCopy,
}: {
  client: KeychainClient;
  itemId: string;
  words: Words;
  settle: (outcome: Refused) => boolean;
  onCopy: (code: string) => void;
}) {
  const [code, setCode] = React.useState<{ code: string; remaining: number } | null>(null);
  const [failed, setFailed] = React.useState(false);
  React.useEffect(() => {
    let current = true;
    let ticker: ReturnType<typeof setInterval> | undefined;
    const ask = async () => {
      if (ticker) clearInterval(ticker);
      const outcome = await asked(client.code(itemId));
      if (!current) return;
      if (!("ok" in outcome)) {
        setCode(null);
        if (!settle(outcome)) setFailed(true);
        return;
      }
      setFailed(false);
      let remaining = outcome.ok.remaining;
      setCode({ code: outcome.ok.code, remaining });
      ticker = setInterval(() => {
        remaining -= 1;
        if (remaining <= 0) return void ask();
        setCode({ code: outcome.ok.code, remaining });
      }, 1000);
    };
    void ask();
    return () => {
      current = false;
      if (ticker) clearInterval(ticker);
      setCode(null);
    };
  }, [client, itemId, settle]);
  if (failed) return <ValueRow title={words.verificationCode} value={words.failed} testID="passwords-detail-code" />;
  if (!code) return <ValueRow title={words.verificationCode} value="…" testID="passwords-detail-code" />;
  return (
    <ValueRow
      title={words.verificationCode}
      value={`${formatCode(code.code)}  ·  ${words.secondsLeft(code.remaining)}`}
      monospace
      accessibilityLabel={`${words.verificationCode}, ${words.codeExpiresIn(code.code.split("").join(" "), code.remaining)}`}
      testID="passwords-detail-code"
    >
      <SmallButton
        label={words.copy}
        icon={RiFileCopyLine}
        accessibilityLabel={words.copyCode}
        onPress={() => onCopy(code.code)}
        testID="passwords-detail-copy-code"
      />
    </ValueRow>
  );
}

/** The editor's fields, as typed. */
type Draft = {
  title: string;
  username: string;
  password: string;
  websites: string;
  code: string;
  notes: string;
  body: string;
};

const EMPTY_DRAFT: Draft = { title: "", username: "", password: "", websites: "", code: "", notes: "", body: "" };

function draftOf(item: ItemView | null): Draft {
  const empty = EMPTY_DRAFT;
  if (!item) return empty;
  if (item.type === "login") {
    return {
      ...empty,
      title: item.title,
      username: item.username,
      password: item.password,
      websites: item.urls.join("\n"),
      notes: item.notes ?? "",
    };
  }
  if (item.type === "note") return { ...empty, title: item.title, body: item.body };
  return empty;
}

/** The item a draft writes: the item it edits kept but for what the editor changes. */
function inputOf(
  draft: Draft,
  type: "login" | "note",
  editing: ItemView | null,
  words: Words,
): { input: ItemInput } | { error: string } {
  const title = draft.title.trim();
  if (title === "") return { error: words.titleRequired };
  if (type === "note") {
    return { input: { type: "note", title, body: draft.body, ...(editing ? { itemId: editing.itemId } : {}) } };
  }
  if (draft.password === "") return { error: words.passwordRequired };
  const kept = editing?.type === "login" ? editing : null;
  let totp = kept?.totp ?? null;
  if (draft.code.trim() !== "") {
    totp = otpauthFrom(draft.code, title, draft.username);
    if (totp === null) return { error: words.invalidCode };
  }
  return {
    input: {
      type: "login",
      title,
      username: draft.username.trim(),
      password: draft.password,
      urls: urlsFromText(draft.websites),
      apps: kept?.apps ?? [],
      totp,
      notes: draft.notes.trim() === "" ? null : draft.notes,
      ...(editing ? { itemId: editing.itemId } : {}),
    },
  };
}

/**
 * An item's sheet: what it holds, then its editor; a new item's editor. The item is asked when the
 * sheet opens and forgotten when it closes; its password shown only when the person asks.
 */
function ItemSheet({
  sheet,
  client,
  words,
  locale,
  audit,
  touch,
  settle,
  onSheet,
  onSaved,
}: {
  sheet: Sheet | null;
  client: KeychainClient;
  words: Words;
  locale: string;
  audit: Audit | null;
  touch: () => void;
  settle: (outcome: Refused) => boolean;
  onSheet: (sheet: Sheet | null) => void;
  onSaved: () => void;
}) {
  const hidden = React.useContext(Hidden);
  const [shown, setShown] = React.useState(false);
  const [draft, setDraft] = React.useState<Draft | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [working, setWorking] = React.useState(false);
  // Deleting: asked first, then the sheet stays hidden until the service answered.
  const [deleting, setDeleting] = React.useState<"asking" | "removing" | null>(null);
  const [copied, setCopied] = React.useState<string | null>(null);
  // Each time the item is asked: as the sheet opens on it, and once it was saved.
  const [revision, setRevision] = React.useState(0);
  const itemId = sheet?.kind === "item" ? sheet.itemId : null;
  // The item, asked once the sheet opens on it and again once saved.
  React.useEffect(() => {
    if (!itemId) return;
    let current = true;
    void asked(client.reveal(itemId)).then((outcome) => {
      if (!current) return;
      if ("ok" in outcome) {
        onSheet({ kind: "item", itemId, item: outcome.ok, editing: false });
      } else if (!settle(outcome)) {
        setError(outcomeMessage(outcome, words));
      }
    });
    return () => {
      current = false;
    };
    // Asked once per opening (the sheet's key changes with each) and per save.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [itemId, revision]);
  const item = sheet?.kind === "item" ? sheet.item : null;
  const editing = sheet?.kind === "new" || (sheet?.kind === "item" && sheet.editing);
  const type: "login" | "note" =
    sheet?.kind === "new" ? sheet.type : item?.type === "note" ? "note" : "login";
  const close = () => {
    if (working) return;
    setShown(false);
    setDraft(null);
    setError(null);
    setCopied(null);
    onSheet(null);
  };
  const copy = async (text: string, sensitive: boolean) => {
    touch();
    const done = await client.copySensitive(text).catch(() => false);
    const said = done ? (sensitive ? words.copied : words.copiedPlain) : words.failed;
    setCopied(said);
    say(said);
  };
  const startEditing = () => {
    if (!item) return;
    touch();
    setDraft(draftOf(item));
    setError(null);
    onSheet({ kind: "item", itemId: item.itemId, item, editing: true });
  };
  const current: Draft = draft ?? EMPTY_DRAFT;
  const change = (key: keyof Draft) => (text: string) => {
    touch();
    setDraft({ ...current, [key]: text });
    setError(null);
  };
  const generate = async () => {
    touch();
    const outcome = await asked(client.generate());
    if ("ok" in outcome) {
      setDraft({ ...current, password: outcome.ok });
      setShown(true);
      say(words.generated);
      return;
    }
    if (!settle(outcome)) setError(outcomeMessage(outcome, words));
  };
  const save = async () => {
    if (working) return;
    const made = inputOf(current, type, sheet?.kind === "item" ? item : null, words);
    if ("error" in made) {
      setError(made.error);
      say(made.error);
      return;
    }
    touch();
    setWorking(true);
    setError(null);
    const outcome = await asked(client.put(made.input));
    setWorking(false);
    if (!("ok" in outcome)) {
      if (settle(outcome)) return;
      const said =
        "refused" in outcome && outcome.refused === "offline" ? words.saveOffline : outcomeMessage(outcome, words);
      setError(said);
      say(said);
      return;
    }
    say(words.saved);
    setDraft(null);
    setShown(false);
    onSaved();
    // The item as the service keeps it now, asked again and shown.
    onSheet({ kind: "item", itemId: outcome.ok, item: null, editing: false });
    setRevision((count) => count + 1);
  };
  const remove = async () => {
    if (!item) return;
    touch();
    const title = titleOf(item);
    const outcome = await asked(client.remove(item.itemId));
    if ("ok" in outcome) {
      say(words.deleted(title));
      onSheet(null);
      setDeleting(null);
      onSaved();
      return;
    }
    if (settle(outcome)) return;
    // Not deleted: the sheet comes back, saying why.
    setDeleting(null);
    const said = outcomeMessage(outcome, words);
    setError(said);
    say(said);
  };
  const title = editing
    ? sheet?.kind === "new"
      ? type === "note"
        ? words.newNote
        : words.newPassword
      : words.editTitle
    : item
      ? titleOf(item)
      : "";
  const warnings = item ? warningsOf(item.itemId, audit) : null;
  const actions: DialogAction[] = editing
    ? [
        {
          label: words.cancel,
          color: "cancel",
          shouldCloseOnPress: sheet?.kind === "new",
          disabled: working,
          onPress: () => {
            if (sheet?.kind === "new") return close();
            setDraft(null);
            setError(null);
            if (item) onSheet({ kind: "item", itemId: item.itemId, item, editing: false });
          },
          testID: "passwords-edit-cancel",
        },
        {
          label: words.save,
          color: "default",
          shouldCloseOnPress: false,
          loading: working,
          onPress: () => void save(),
          testID: "passwords-edit-save",
        },
      ]
    : [
        ...(item && item.type !== "passkey"
          ? [
              {
                label: words.delete,
                color: "destructive",
                shouldCloseOnPress: false,
                onPress: () => {
                  touch();
                  setDeleting("asking");
                },
                testID: "passwords-detail-delete",
              } as DialogAction,
            ]
          : []),
        ...(item && (item.type === "login" || item.type === "note")
          ? [
              {
                label: words.edit,
                color: "cancel",
                shouldCloseOnPress: false,
                onPress: startEditing,
                testID: "passwords-detail-edit",
              } as DialogAction,
            ]
          : []),
        { label: words.done, color: "default", onPress: close, testID: "passwords-detail-done" },
      ];
  const passwordRow = (password: string) => (
    <ValueRow
      title={words.password}
      value={shown ? password : words.hidden}
      monospace={shown}
      accessibilityLabel={shown ? `${words.password}, ${password}` : `${words.password}, ${words.hiddenLabel}`}
      testID="passwords-detail-password"
    >
      <SmallButton
        label={shown ? words.hide : words.show}
        accessibilityLabel={shown ? words.hidePassword : words.showPassword}
        onPress={() => {
          touch();
          setShown((now) => !now);
        }}
        testID="passwords-detail-show"
      />
      <SmallButton
        label={words.copy}
        icon={RiFileCopyLine}
        accessibilityLabel={words.copyPassword}
        onPress={() => void copy(password, true)}
        testID="passwords-detail-copy-password"
      />
    </ValueRow>
  );
  const detail = (() => {
    if (!item) {
      return error ? (
        <ErrorLine text={error} testID="passwords-detail-error" />
      ) : (
        <View style={{ alignItems: "center", paddingVertical: 16 }}>
          <Loading size="sm" accessibilityLabel={words.title} />
        </View>
      );
    }
    const rows: React.ReactNode[] = [];
    switch (item.type) {
      case "login":
        rows.push(
          <ValueRow key="user" title={words.username} value={item.username || words.noUsername} testID="passwords-detail-username">
            {item.username ? (
              <SmallButton
                label={words.copy}
                icon={RiFileCopyLine}
                accessibilityLabel={words.copyUsername}
                onPress={() => void copy(item.username, false)}
                testID="passwords-detail-copy-username"
              />
            ) : null}
          </ValueRow>,
          <React.Fragment key="password">{passwordRow(item.password)}</React.Fragment>,
        );
        if (item.totp) {
          rows.push(
            <LiveCode
              key="code"
              client={client}
              itemId={item.itemId}
              words={words}
              settle={settle}
              onCopy={(code) => void copy(code, true)}
            />,
          );
        }
        if (item.urls.length > 0) {
          rows.push(
            <ValueRow key="sites" title={words.websites} value={item.urls.join("\n")} testID="passwords-detail-websites" />,
          );
        }
        if (item.notes) rows.push(<ValueRow key="notes" title={words.notes} value={item.notes} testID="passwords-detail-notes" />);
        break;
      case "passkey":
        rows.push(
          <ValueRow key="rp" title={words.relyingParty} value={item.rpId} testID="passwords-detail-rp" />,
          <ValueRow key="user" title={words.username} value={item.userName} testID="passwords-detail-username" />,
          <ValueRow key="name" title={words.displayName} value={item.userDisplayName} testID="passwords-detail-display-name" />,
        );
        break;
      case "totp":
        rows.push(
          <LiveCode
            key="code"
            client={client}
            itemId={item.itemId}
            words={words}
            settle={settle}
            onCopy={(code) => void copy(code, true)}
          />,
        );
        if (item.account) rows.push(<ValueRow key="account" title={words.username} value={item.account} testID="passwords-detail-username" />);
        if (item.notes) rows.push(<ValueRow key="notes" title={words.notes} value={item.notes} testID="passwords-detail-notes" />);
        break;
      case "note":
        rows.push(<ValueRow key="body" title={words.body} value={item.body} testID="passwords-detail-body" />);
        break;
      case "wifi":
        rows.push(<ValueRow key="ssid" title={words.network} value={item.ssid} testID="passwords-detail-network" />);
        if (item.password) rows.push(<React.Fragment key="password">{passwordRow(item.password)}</React.Fragment>);
        rows.push(
          <ValueRow key="security" title={words.networkSecurity} value={item.security} testID="passwords-detail-security" />,
        );
        break;
    }
    return (
      <View style={{ gap: 4, paddingBottom: 12 }}>
        {warnings && (warnings.weak || warnings.reusedWith > 0) ? (
          <View style={{ flexDirection: "row", gap: 8 }} testID="passwords-detail-warning">
            <RiErrorWarningLine width={18} height={18} fill="#d97706" />
            <Text variant="body-2-regular" style={{ flex: 1 }}>
              {`${
                warnings.weak && warnings.reusedWith > 0
                  ? words.reusedAndWeak(warnings.reusedWith)
                  : warnings.weak
                    ? words.weak
                    : words.reused(warnings.reusedWith)
              }. ${words.changePassword}`}
            </Text>
          </View>
        ) : null}
        {rows}
        <Text variant="caption-1-regular" style={{ opacity: 0.7, paddingTop: 8 }} testID="passwords-detail-modified">
          {words.modified(formatDate(item.modifiedAt, locale))}
        </Text>
        <ErrorLine text={error} testID="passwords-detail-error" />
        {copied ? (
          <Text variant="caption-1-regular" testID="passwords-detail-copied">
            {copied}
          </Text>
        ) : null}
      </View>
    );
  })();
  const editor = (
    <View style={{ gap: 8, paddingBottom: 12 }}>
      <Field label={words.title_} value={current.title} onChange={change("title")} autoFocus testID="passwords-edit-title" />
      {type === "login" ? (
        <>
          <Field label={words.username} value={current.username} onChange={change("username")} testID="passwords-edit-username" />
          <View style={{ flexDirection: "row", gap: 8, alignItems: "center" }}>
            <View style={{ flex: 1 }}>
              {/* Always a secure field: copying from one does nothing (docs/adr/0007-clipboard.mdx,
                  decision 6), so a password leaves only by Copy, marked sensitive. Shown, it is
                  drawn under the field as text no one can select. */}
              <Field
                label={words.password}
                value={current.password}
                onChange={change("password")}
                secure
                testID="passwords-edit-password"
              />
            </View>
            <SmallButton
              label={shown ? words.hide : words.show}
              accessibilityLabel={shown ? words.hidePassword : words.showPassword}
              onPress={() => setShown((now) => !now)}
              testID="passwords-edit-show"
            />
          </View>
          {shown && current.password !== "" ? (
            <Text
              variant="body-regular"
              selectable={false}
              style={{ fontFamily: "monospace" }}
              accessibilityLabel={`${words.password}, ${current.password}`}
              testID="passwords-edit-password-shown"
            >
              {current.password}
            </Text>
          ) : null}
          <Button
            size="sm"
            appearance="plain"
            tone="neutral"
            icon={RiKey2Line}
            onPress={() => void generate()}
            testID="passwords-edit-generate"
          >
            {words.generate}
          </Button>
          <Field
            label={words.websites}
            value={current.websites}
            onChange={change("websites")}
            multiline
            testID="passwords-edit-websites"
          />
          <TextFieldHint>{words.websitesHint}</TextFieldHint>
          {item?.type === "login" && item.totp ? null : (
            <>
              <Field label={words.setUpCode} value={current.code} onChange={change("code")} testID="passwords-edit-code" />
              <TextFieldHint>{words.setUpCodeHint}</TextFieldHint>
            </>
          )}
          <Field label={words.notes} value={current.notes} onChange={change("notes")} multiline testID="passwords-edit-notes" />
        </>
      ) : (
        <Field label={words.body} value={current.body} onChange={change("body")} multiline testID="passwords-edit-body" />
      )}
      <ErrorLine text={error} testID="passwords-edit-error" />
    </View>
  );
  return (
    <>
      <Dialog
        open={sheet !== null && deleting === null && !hidden}
        onClose={close}
        title={title}
        label={title}
        actions={actions}
        scrollable
        dismissOnBackdrop={!editing && !working}
        testID="passwords-sheet"
      >
        {editing ? editor : detail}
      </Dialog>
      <AlertDialog
        open={deleting === "asking" && !hidden}
        onOpenChange={(open) => !open && setDeleting((now) => (now === "asking" ? null : now))}
        title={words.deleteTitle(item ? titleOf(item) : "")}
        description={words.deleteDescription}
        confirmLabel={words.deleteConfirm}
        cancelLabel={words.cancel}
        destructive
        onConfirm={() => {
          setDeleting("removing");
          void remove();
        }}
        // Cancelled, the item's sheet comes back, as a Mac's alert returns to its sheet.
        onCancel={() => setDeleting(null)}
        testID="passwords-delete-confirm"
      />
    </>
  );
}

function titleOf(item: ItemView): string {
  switch (item.type) {
    case "passkey":
      return item.rpName || item.rpId;
    case "wifi":
      return item.ssid;
    default:
      return item.title;
  }
}

function formatDate(iso: string, locale: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  try {
    return new Intl.DateTimeFormat(locale, { dateStyle: "medium", timeStyle: "short" }).format(date);
  } catch {
    return date.toISOString();
  }
}

/**
 * Devices (decision 4): the members that hold the keychain, each but this one removable (the
 * vault's key changes), and the devices waiting to join. Approving shows the code the other device
 * shows; it is approved only once the person says they match.
 */
function Devices({
  client,
  words,
  touch,
  settle,
}: {
  client: KeychainClient;
  words: Words;
  touch: () => void;
  settle: (outcome: Refused) => boolean;
}) {
  const hidden = React.useContext(Hidden);
  const [members, setMembers] = React.useState<Member[] | null>(null);
  const [pending, setPending] = React.useState<Enrollment[]>([]);
  const [error, setError] = React.useState<string | null>(null);
  const [removing, setRemoving] = React.useState<Member | null>(null);
  const [approving, setApproving] = React.useState<Enrollment | null>(null);
  const load = React.useCallback(async () => {
    const [listed, waiting] = await Promise.all([asked(client.members()), asked(client.enrollments())]);
    if ("ok" in listed) setMembers(listed.ok);
    else if (!settle(listed)) setError(outcomeMessage(listed, words));
    if ("ok" in waiting) setPending(waiting.ok.filter((enrollment) => enrollment.status !== "approved"));
  }, [client, settle, words]);
  React.useEffect(() => {
    void load();
    // A device asking to join appears while the page is open.
    const timer = setInterval(() => void load(), 5000);
    return () => clearInterval(timer);
  }, [load]);
  const remove = async (member: Member) => {
    touch();
    const outcome = await asked(client.removeMember(member.memberId));
    if ("ok" in outcome) {
      say(words.deviceRemoved(member.name));
    } else if (!settle(outcome)) {
      const said = outcomeMessage(outcome, words);
      setError(said);
      say(said);
    }
    await load();
  };
  const deny = async (enrollment: Enrollment) => {
    touch();
    const outcome = await asked(client.deny(enrollment.enrollmentId));
    if (!("ok" in outcome) && !settle(outcome)) setError(outcomeMessage(outcome, words));
    await load();
  };
  const nameOf = (enrollment: Enrollment) => enrollment.name || words.aDevice;
  return (
    <View style={{ gap: 16 }} testID="passwords-devices">
      <Text variant="body-2-regular">{words.devicesDescription}</Text>
      <GroupedSection>
        {members === null ? (
          <GroupedSectionItem title="…" testID="passwords-members-loading" />
        ) : (
          members.map((member) => (
            <GroupedSectionItem
              key={member.memberId}
              icon={<RiComputerLine width={20} height={20} />}
              title={member.name}
              description={member.self ? words.thisDevice : member.orphaned ? words.orphaned : member.platform}
              descriptionLines={0}
              testID={`passwords-member-${member.memberId}`}
              rightElement={
                member.self ? null : (
                  <SmallButton
                    label={words.removeDevice}
                    accessibilityLabel={words.removeDeviceLabel(member.name)}
                    onPress={() => {
                      touch();
                      setRemoving(member);
                    }}
                    testID={`passwords-member-remove-${member.memberId}`}
                  />
                )
              }
            />
          ))
        )}
      </GroupedSection>
      <GroupedSection title={words.pendingTitle}>
        {pending.length === 0 ? (
          <GroupedSectionItem title={words.noPending} testID="passwords-pending-none" />
        ) : (
          pending.map((enrollment) => (
            <GroupedSectionItem
              key={enrollment.enrollmentId}
              icon={<RiComputerLine width={20} height={20} />}
              title={nameOf(enrollment)}
              description={enrollment.platform}
              testID={`passwords-pending-${enrollment.enrollmentId}`}
              rightElement={
                <View style={{ flexDirection: "row", gap: 8 }}>
                  <SmallButton
                    label={words.deny}
                    accessibilityLabel={words.denyLabel(nameOf(enrollment))}
                    onPress={() => void deny(enrollment)}
                    testID={`passwords-pending-deny-${enrollment.enrollmentId}`}
                  />
                  <SmallButton
                    label={words.approve}
                    accessibilityLabel={words.approveLabel(nameOf(enrollment))}
                    onPress={() => {
                      touch();
                      setApproving(enrollment);
                    }}
                    testID={`passwords-pending-approve-${enrollment.enrollmentId}`}
                  />
                </View>
              }
            />
          ))
        )}
      </GroupedSection>
      <ErrorLine text={error} testID="passwords-devices-error" />
      <AlertDialog
        open={removing !== null && !hidden}
        onOpenChange={(open) => !open && setRemoving(null)}
        title={words.removeDeviceTitle(removing?.name ?? "")}
        description={words.removeDeviceDescription}
        confirmLabel={words.removeDeviceConfirm}
        cancelLabel={words.cancel}
        destructive
        onConfirm={() => {
          const member = removing;
          setRemoving(null);
          if (member) void remove(member);
        }}
        onCancel={() => setRemoving(null)}
        testID="passwords-member-remove-confirm"
      />
      {approving ? (
        <ApproveSheet
          enrollment={approving}
          name={nameOf(approving)}
          client={client}
          words={words}
          touch={touch}
          settle={settle}
          onDone={() => {
            setApproving(null);
            void load();
          }}
        />
      ) : null}
    </View>
  );
}

/** Approving a device (decision 4): its code, once it showed its own, and approved only once the
    person confirms both match. */
function ApproveSheet({
  enrollment,
  name,
  client,
  words,
  touch,
  settle,
  onDone,
}: {
  enrollment: Enrollment;
  name: string;
  client: KeychainClient;
  words: Words;
  touch: () => void;
  settle: (outcome: Refused) => boolean;
  onDone: () => void;
}) {
  const hidden = React.useContext(Hidden);
  const [code, setCode] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [working, setWorking] = React.useState(false);
  React.useEffect(() => {
    let current = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const follow = async () => {
      const outcome = await asked(client.approve(enrollment.enrollmentId));
      if (!current) return;
      if ("ok" in outcome) {
        if (outcome.ok.code) {
          setCode(outcome.ok.code);
          say(`${words.approveTitle(name)} ${words.joiningCode(outcome.ok.code.split("").join(" "))}`);
          return;
        }
        timer = setTimeout(() => void follow(), FOLLOW_MS);
        return;
      }
      if (settle(outcome)) return;
      const said = outcomeMessage(outcome, words);
      setError(said);
      say(said);
    };
    void follow();
    return () => {
      current = false;
      if (timer) clearTimeout(timer);
    };
  }, [client, enrollment.enrollmentId, name, words, settle]);
  const confirm = async () => {
    if (working || !code) return;
    touch();
    setWorking(true);
    const outcome = await asked(client.confirmApproval(enrollment.enrollmentId));
    setWorking(false);
    if ("ok" in outcome) {
      say(words.approved(name));
      return onDone();
    }
    if (settle(outcome)) return;
    const said = outcomeMessage(outcome, words);
    setError(said);
    say(said);
  };
  const deny = async () => {
    touch();
    await asked(client.deny(enrollment.enrollmentId));
    onDone();
  };
  return (
    <Dialog
      open={!hidden}
      onClose={onDone}
      title={words.approveTitle(name)}
      description={code ? words.approveDescription(name) : words.approveWaiting}
      label={words.approveTitle(name)}
      actions={[
        { label: words.cancel, color: "cancel", onPress: onDone, disabled: working, testID: "passwords-approve-cancel" },
        {
          label: words.deny,
          color: "destructive",
          shouldCloseOnPress: false,
          disabled: working,
          onPress: () => void deny(),
          testID: "passwords-approve-deny",
        },
        {
          label: words.codesMatch,
          color: "default",
          shouldCloseOnPress: false,
          loading: working,
          disabled: code === null,
          onPress: () => void confirm(),
          testID: "passwords-approve-confirm",
        },
      ]}
      dismissOnBackdrop={!working}
      testID="passwords-approve-sheet"
    >
      <View style={{ gap: 8, paddingBottom: 12, alignItems: "center" }}>
        {code ? (
          <Text
            variant="title-3-semibold"
            style={{ fontFamily: "monospace", letterSpacing: 4 }}
            accessibilityLabel={words.joiningCode(code.split("").join(" "))}
            testID="passwords-approve-code"
          >
            {formatCode(code)}
          </Text>
        ) : (
          <Loading size="sm" accessibilityLabel={words.approveWaiting} />
        )}
        <ErrorLine text={error} testID="passwords-approve-error" />
      </View>
    </Dialog>
  );
}
