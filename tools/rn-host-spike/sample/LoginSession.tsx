/**
 * OxyOS's login screen (apps/login, docs/adr/0020-oxy-account-login.mdx) as the Bloom host runs it
 * at the login program's seat: the host run as the login screen's shell (`--shell`, by
 * `oxyos-session --greeter`), the compositor locked from the start. Its root, the output's
 * background, is the wallpaper; the login screen is a lock surface of its own, the only thing the
 * compositor shows and reaches. The accounts are the accounts' service's; the password goes
 * through the compositor to the login program, never further into JavaScript. Adding an account
 * runs the Oxy SDK's own sign-in (SetupSignIn.tsx) and hands the credential and the password to
 * the accounts' service through the host (`OxyOSAccountSession`), never keeping either here. At the
 * first setup, Migrate from a Backup asks the migration service through the host (`OxyOSMigration`,
 * docs/adr/0059-backups.mdx), the backup password going straight on to it.
 */
import * as React from 'react';
import { StyleSheet, TurboModuleRegistry, View } from 'react-native';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { oxyosSurfaces } from './oxyos/platform-surface';
import { oxyosShell } from './oxyos/shell-surface';
import { useOxyOSShell } from './oxyos/shell-state';
import {
  LoginScreen,
  type LoginActions,
  type MigrationActions,
  type SetupActions,
} from '../../../apps/login/src/LoginScreen';
import { accountsFrom, type Accounts } from '../../../apps/login/src/model';
import { enrollAnswerFrom, platformFrom } from '../../../apps/login/src/setup';
import { SetupSignIn } from './SetupSignIn';
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
  enroll(password: string): Promise<unknown>;
  secondFactor(code: string): Promise<unknown>;
  endEnrollment(): void;
}>('OxyOSAccountSession');

/** Setting an account up through the host: the accounts' service's platform and its setup. */
const setupActions: SetupActions | undefined =
  accountSession && accountSession.getConstants().setup
    ? {
        platform: () => platformFrom(accountSession.platform()),
        SignIn: SetupSignIn,
        enroll: async (password) => enrollAnswerFrom(await accountSession.enroll(password)),
        secondFactor: async (code) => enrollAnswerFrom(await accountSession.secondFactor(code)),
        end: () => accountSession.endEnrollment(),
      }
    : undefined;

const migrationModule = TurboModuleRegistry.get<{
  getConstants(): { available: boolean };
  disks(): Promise<unknown>;
  backups(source: object, password: string, sharePassword: string | null): Promise<unknown>;
  migrate(repository: string, snapshot: string, progress: (told: unknown) => void): Promise<unknown>;
  end(): void;
}>('OxyOSMigration');

/** Migrating from a backup through the host: the migration service's conversation. */
const migrationActions: MigrationActions | undefined =
  migrationModule && migrationModule.getConstants().available
    ? {
        disks: () => migrationModule.disks(),
        backups: (source, password, sharePassword) => migrationModule.backups(source, password, sharePassword),
        migrate: (repository, snapshot, progress) => migrationModule.migrate(repository, snapshot, progress),
        end: () => migrationModule.end(),
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
  // The catalog's default, as the login screen's session tells it (docs/adr/0048-wallpapers.mdx).
  const wallpaper = useShellWallpaper(state?.wallpaper);
  const actions: LoginActions = {
    authenticate: (account, password) => shell.authenticate(password, account),
    answer: shell.authentication ?? undefined,
    capsLock: !!state?.capsLock,
    online: (api) => reachable(api ?? state?.oxyApi ?? 'https://api.oxy.so'),
    setup: setupActions,
    migration: migrationActions,
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
              <LoginScreen accounts={accounts} actions={actions} wallpaper={wallpaper} now={now} Masked={Masked} />
            </Surface>
          )}
        </View>
      </BloomProvider>
    </SafeAreaProvider>
  );
}

const styles = StyleSheet.create({ fill: { flex: 1, backgroundColor: '#000' } });
