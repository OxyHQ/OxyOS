/**
 * OxyOS's login screen (apps/login, docs/adr/0020-oxy-account-login.mdx) as the Bloom host runs it
 * at the login program's seat: the host run as the login screen's shell (`--shell`, by
 * `oxyos-session --greeter`), the compositor locked from the start. Its root, the output's
 * background, is the wallpaper; the login screen is a lock surface of its own, the only thing the
 * compositor shows and reaches. The accounts are the accounts' service's; the password goes
 * through the compositor to the login program, never further into JavaScript.
 */
import * as React from 'react';
import { StyleSheet, View } from 'react-native';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { oxyosSurfaces } from './oxyos/platform-surface';
import { oxyosShell } from './oxyos/shell-surface';
import { useOxyOSShell } from './oxyos/shell-state';
import { LoginScreen, type LoginActions } from '../../../apps/login/src/LoginScreen';
import { accountsFrom, type Accounts } from '../../../apps/login/src/model';
import { Wallpaper } from '../../../apps/shell/src/components/Wallpaper';
import { useMinuteClock } from '../../../apps/shell/src/useMinuteClock';

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

export default function LoginSession() {
  const shell = useOxyOSShell();
  const now = useMinuteClock();
  const state = shell.state;
  const [accounts, setAccounts] = React.useState<Accounts>(() => accountsFrom(shell.accounts()));
  // Accounts that cannot be read are asked for again: nobody signs in until they can.
  React.useEffect(() => {
    if (accounts !== null) return undefined;
    const timer = setTimeout(() => setAccounts(accountsFrom(shell.accounts())), RETRY_MS);
    return () => clearTimeout(timer);
  }, [accounts, shell]);
  const wallpaper = state?.wallpaper
    ? { uri: state.wallpaper }
    : require('../../../assets/wallpapers/solar-gradients/frames/frame-08.jpg');
  const actions: LoginActions = {
    authenticate: (account, password) => shell.authenticate(password, account),
    answer: shell.authentication ?? undefined,
    capsLock: !!state?.capsLock,
    online: () => reachable(state?.oxyApi ?? 'https://api.oxy.so'),
  };
  const { Surface, Masked } = oxyosShell;
  return (
    <SafeAreaProvider>
      <BloomProvider mode="dark" colorPreset="graphite" haptics={false} locale="es" platform={oxyosSurfaces}>
        <View testID="login-background" style={styles.fill}>
          <Wallpaper source={wallpaper} decorative />
          {state?.locked && (
            <Surface kind="lock" style={StyleSheet.absoluteFill}>
              <LoginScreen accounts={accounts} actions={actions} wallpaper={wallpaper} now={now} Masked={Masked} />
            </Surface>
          )}
        </View>
      </BloomProvider>
    </SafeAreaProvider>
  );
}

const styles = StyleSheet.create({ fill: { flex: 1, backgroundColor: '#000' } });
