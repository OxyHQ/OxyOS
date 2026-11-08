/**
 * The login screen's setup sign-in (docs/adr/0020-oxy-account-login.mdx, decision 4): the Oxy SDK's
 * own account chooser, the one every Oxy app's account dialog shows (`OxyAuthChooser` from
 * `@oxy.so/services`), bare on the login screen's card (`OxySurfaceFrame`, which the chooser sizes
 * as it sizes the dialog: its split card, with the Commons QR a phone scans, from `md`), inside
 * `OxyProvider` with OxyOS's client id and Oxy's API as the accounts' service names them, and the
 * SDK's seam for a platform's setup client, `authStateStore` (oxy ADR 0058, B3): a store that keeps
 * nothing and hands the credential a sign-in commits to the Bloom host at once
 * (`OxyOSAccountSession.holdCredential`), which holds it until the setup ends. Every way in is the
 * SDK's: the email's code or link, the password, the authenticator, the QR approved on a phone, or
 * a new account. Nothing here is a sign-in of OxyOS's own.
 *
 * Once signed in, the account's sign-in methods say whether it has a password (decision 2): with
 * one, it is asked once more; without, the person chooses a PIN and is never asked to make an Oxy
 * password. Methods that cannot be read are never taken for "no password": the screen says so and
 * asks again, and nothing goes on until they are read (failing closed).
 */
import * as React from 'react';
import { Pressable, Text, TurboModuleRegistry, View } from 'react-native';
import { OxyAuthChooser, OxyProvider, OxySurfaceFrame, useOxy } from '@oxy.so/services';
import type { SetupSignInProps } from '../../../apps/login/src/LoginScreen';
import { useLoginMessages } from '../../../apps/login/src/useLoginMessages';

const native = TurboModuleRegistry.get<{
  holdCredential(deviceId: string, deviceSecret: string, accountId: string): boolean;
}>('OxyOSAccountSession');

type Persisted = { sessionId: string; userId: string; deviceId?: string; deviceSecret?: string };

/** The setup's store: nothing kept, nothing loaded; a committed credential goes to the host. */
function handingOverStore(onHanded: (accountId: string) => void) {
  return {
    load: async () => null,
    save: async (state: Persisted) => {
      if (!state.deviceId || !state.deviceSecret || !state.userId || !native) return false;
      const held = native.holdCredential(state.deviceId, state.deviceSecret, state.userId);
      if (held) onHanded(state.userId);
      return held;
    },
    clear: async () => {},
  };
}

/** Whether the account has a password: read from its sign-in methods, or not known. */
type Methods = { state: 'reading' } | { state: 'read'; hasPassword: boolean } | { state: 'unread' };

/**
 * After the sign-in: the account's sign-in methods read, then the setup's next step with whether
 * it has a password. Not read (offline, refused, an answer outside the contract), it says so and
 * offers to read them again; it never guesses.
 */
function AfterSignIn({ accountId, onSignedIn }: { accountId: string; onSignedIn: SetupSignInProps['onSignedIn'] }) {
  const words = useLoginMessages().setup;
  const { oxyServices, user } = useOxy() as {
    oxyServices: { auth: { methods(): Promise<{ hasPassword?: unknown }> } };
    user?: { username?: string; name?: { full?: string } | string } | null;
  };
  const [methods, setMethods] = React.useState<Methods>({ state: 'reading' });
  const [attempt, setAttempt] = React.useState(0);
  const name = (typeof user?.name === 'string' ? user.name : user?.name?.full) || user?.username || '';
  React.useEffect(() => {
    let current = true;
    setMethods({ state: 'reading' });
    oxyServices.auth
      .methods()
      .then((read) => {
        if (!current) return;
        setMethods(
          typeof read?.hasPassword === 'boolean' ? { state: 'read', hasPassword: read.hasPassword } : { state: 'unread' },
        );
      })
      .catch(() => current && setMethods({ state: 'unread' }));
    return () => {
      current = false;
    };
  }, [oxyServices, attempt]);
  React.useEffect(() => {
    if (methods.state === 'read') onSignedIn({ id: accountId, name, hasPassword: methods.hasPassword });
  }, [methods, accountId, name, onSignedIn]);
  if (methods.state === 'read') return null;
  return (
    <View testID="login-setup-methods" style={{ alignItems: 'center', paddingVertical: 16 }}>
      <Text accessibilityRole="alert" style={{ color: '#fff', fontSize: 13, textAlign: 'center' }}>
        {methods.state === 'reading' ? words.checkingAccount : words.accountUnchecked}
      </Text>
      {methods.state === 'unread' ? (
        <Pressable
          testID="login-setup-methods-retry"
          accessibilityRole="button"
          accessibilityLabel={words.retry}
          onPress={() => setAttempt((count) => count + 1)}
          style={{ marginTop: 14, paddingHorizontal: 12, paddingVertical: 4 }}
        >
          <Text style={{ color: '#fff', fontSize: 13, fontWeight: '600' }}>{words.retry}</Text>
        </Pressable>
      ) : null}
    </View>
  );
}

/**
 * The chooser until a sign-in signed the account in, whichever way it came (the chooser's own
 * steps, or the QR the provider's controller completes): the account the host was handed the
 * credential of, now the provider's. Then what it signs in with on this computer.
 */
function Setup({
  accountId,
  onSignedIn,
  onFrameWidth,
}: {
  accountId: string | null;
  onSignedIn: SetupSignInProps['onSignedIn'];
  onFrameWidth: SetupSignInProps['onFrameWidth'];
}) {
  const { user } = useOxy() as { user?: { id?: string } | null };
  if (accountId && user?.id === accountId) {
    return <AfterSignIn accountId={accountId} onSignedIn={onSignedIn} />;
  }
  return (
    <OxySurfaceFrame onMaxWidthChange={onFrameWidth}>
      <OxyAuthChooser />
    </OxySurfaceFrame>
  );
}

export function SetupSignIn({ platform, onSignedIn, onFrameWidth }: SetupSignInProps) {
  const [accountId, setAccountId] = React.useState<string | null>(null);
  const store = React.useMemo(() => handingOverStore(setAccountId), []);
  return (
    <View testID="login-setup-sign-in" style={{ width: '100%' }}>
      <OxyProvider baseURL={platform.api} clientId={platform.clientId} authStateStore={store}>
        <Setup accountId={accountId} onSignedIn={onSignedIn} onFrameWidth={onFrameWidth} />
      </OxyProvider>
    </View>
  );
}
