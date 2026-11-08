/**
 * OxyOS's login screen (apps/login, docs/adr/0020-oxy-account-login.mdx) as the Bloom host runs it
 * at the login program's seat: the host run as the login screen's shell (`--shell`, by
 * `oxyos-session --greeter`), the compositor locked from the start. Its root, the output's
 * background, is the wallpaper; the login screen is a lock surface of its own, the only thing the
 * compositor shows and reaches. The accounts are the accounts' service's; the password goes
 * through the compositor to the login program, never further into JavaScript; a PIN likewise, said
 * as one. Adding an account is the setup assistant in the Oxy SDK's own frame with its own sign-in
 * (SetupAssistant.tsx), which hands the credential, the password and the PIN chosen to the
 * accounts' service through the host (`OxyOSAccountSession`), never keeping any of them here, the
 * service's stages told as they come (`OxyOSAccountSetupStage`).
 */
import * as React from 'react';
import { DeviceEventEmitter, StyleSheet, TurboModuleRegistry, View } from 'react-native';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { oxyosSurfaces } from './oxyos/platform-surface';
import { oxyosShell } from './oxyos/shell-surface';
import { useOxyOSShell } from './oxyos/shell-state';
import { LoginScreen, type LoginActions, type SetupActions } from '../../../apps/login/src/LoginScreen';
import { accountsFrom, type Accounts } from '../../../apps/login/src/model';
import { enrollAnswerFrom, platformFrom, stageFrom, type SetupStage } from '../../../apps/login/src/setup';
import { SetupAssistantFrame } from './SetupAssistant';
import { Wallpaper } from '../../../apps/shell/src/components/Wallpaper';
import { useMinuteClock } from '../../../apps/shell/src/useMinuteClock';
import { useSessionLocale } from './oxyos/locale';
import { useShellWallpaper } from './oxyos/wallpaper';

/** How long the network may take to answer before setup says it needs it. */
const REACH_MS = 5000;
/** Unreadable accounts are asked for again this often. */
const RETRY_MS = 5000;

/** Whether Oxy's API answers at all: any answer is the network, a failure none. */
async function reachable(api: string): Promise<boolean> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), REACH_MS);
  try {
    await fetch(api, { method: 'GET', signal: controller.signal });
    return true;
  } catch {
    return false;
  } finally {
    clearTimeout(timer);
  }
}

const accountSession = TurboModuleRegistry.get<{
  getConstants(): { setup: boolean };
  platform(): unknown;
  enroll(password: string | null, pin: string | null, appearance: string | null): Promise<unknown>;
  secondFactor(code: string): Promise<unknown>;
  retryEnrollment(): Promise<unknown>;
  endEnrollment(): void;
}>('OxyOSAccountSession');

/** A call of the setup, its stages told to `onStage` while it runs. */
async function withStages(call: () => Promise<unknown>, onStage: (stage: SetupStage) => void) {
  const listener = DeviceEventEmitter.addListener('OxyOSAccountSetupStage', (event: { stage?: unknown }) => {
    const stage = stageFrom(event?.stage);
    if (stage) onStage(stage);
  });
  try {
    return enrollAnswerFrom(await call());
  } finally {
    listener.remove();
  }
}

/** Setting an account up through the host: the accounts' service's platform and its setup. */
const setupActions: SetupActions | undefined =
  accountSession && accountSession.getConstants().setup
    ? {
        platform: () => platformFrom(accountSession.platform()),
        Assistant: SetupAssistantFrame,
        enroll: ({ password, pin, appearance }, onStage) => {
          // What the setup sends, for the log; never the secrets.
          console.info(`[setup] enroll with ${password ? 'the password' : 'no password'} and ${pin ? 'a PIN' : 'no PIN'}, ${appearance}`);
          return withStages(() => accountSession.enroll(password, pin, appearance), onStage);
        },
        secondFactor: (code, onStage) => withStages(() => accountSession.secondFactor(code), onStage),
        retry: (onStage) => {
          console.info('[setup] try again with what the service kept');
          return withStages(() => accountSession.retryEnrollment(), onStage);
        },
        end: () => accountSession.endEnrollment(),
      }
    : undefined;

export default function LoginSession() {
  const shell = useOxyOSShell();
  const now = useMinuteClock();
  // The login screen's locale is the system's alone (docs/adr/0040-language-and-region.mdx).
  const locale = useSessionLocale();
  const state = shell.state;
  const [accounts, setAccounts] = React.useState<Accounts>(() => accountsFrom(shell.accounts()));
  // Accounts that cannot be read are asked for again: nobody signs in until they can.
  React.useEffect(() => {
    if (accounts !== null) return undefined;
    const timer = setTimeout(() => setAccounts(accountsFrom(shell.accounts())), RETRY_MS);
    return () => clearTimeout(timer);
  }, [accounts, shell]);
  // A rejected attempt may have locked a PIN out: the accounts are read again, so the screen
  // offers the password (or Oxy) in its place.
  const answer = shell.authentication;
  React.useEffect(() => {
    if (answer?.outcome === 'rejected') setAccounts(accountsFrom(shell.accounts()));
  }, [answer?.id]);
  // The catalog's default, as the login screen's session tells it (docs/adr/0048-wallpapers.mdx).
  const wallpaper = useShellWallpaper(state?.wallpaper);
  const actions: LoginActions = {
    authenticate: (account, password) => shell.authenticate(password, account),
    authenticatePin: (account, pin) => shell.authenticatePin(pin, account),
    answer: shell.authentication ?? undefined,
    capsLock: !!state?.capsLock,
    online: (api) => reachable(api ?? state?.oxyApi ?? 'https://api.oxy.so'),
    setup: setupActions,
    reloadAccounts: () => setAccounts(accountsFrom(shell.accounts())),
    announce: (text, polite) => shell.announce(text, polite),
  };
  const { Surface, Masked } = oxyosShell;
  return (
    <SafeAreaProvider>
      <BloomProvider mode="dark" colorPreset="graphite" haptics={false} locale={locale} platform={oxyosSurfaces}>
        <View testID="login-background" style={styles.fill}>
          <Wallpaper wallpaper={wallpaper} decorative />
          {state?.locked && (
            <Surface kind="lock" style={StyleSheet.absoluteFill}>
              <LoginScreen
                accounts={accounts}
                actions={actions}
                wallpaper={wallpaper}
                now={now}
                Masked={Masked}
                safeMode={state?.safeMode === true}
              />
            </Surface>
          )}
        </View>
      </BloomProvider>
    </SafeAreaProvider>
  );
}

const styles = StyleSheet.create({ fill: { flex: 1, backgroundColor: '#000' } });
