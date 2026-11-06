/**
 * The login screen's setup sign-in (docs/adr/0020-oxy-account-login.mdx, decision 4): the Oxy SDK's
 * own sign-in, `OxyProvider` from `@oxy.so/services` with OxyOS's client id and Oxy's API as the
 * accounts' service names them, and the SDK's seam for a platform's setup client, `authStateStore`
 * (oxy ADR 0058, B3): a store that keeps nothing and hands the credential a sign-in commits to the
 * Bloom host at once (`OxyOSAccountSession.holdCredential`), which holds it until the person gives
 * the account's password once more. An account without a password sets one through the SDK's own
 * flow (`OxyPasswordPanel`) before that; a person without an account creates one with the SDK's
 * own (`OxySignUpPanel`). Nothing here is a sign-in of OxyOS's own.
 */
import * as React from 'react';
import { TurboModuleRegistry, View } from 'react-native';
import { OxyPasswordPanel, OxyProvider, OxySignInPanel, OxySignUpPanel, useOxy } from '@oxy.so/services';
import type { SetupSignInProps } from '../../../apps/login/src/LoginScreen';

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

/** After the sign-in: the account's password set if it has none, then the setup's next step. */
function AfterSignIn({ accountId, onSignedIn }: { accountId: string; onSignedIn: SetupSignInProps['onSignedIn'] }) {
  const { oxyServices, user } = useOxy() as {
    oxyServices: { auth: { methods(): Promise<{ hasPassword: boolean }> } };
    user?: { username?: string; name?: { full?: string } | string } | null;
  };
  const [hasPassword, setHasPassword] = React.useState<boolean | null>(null);
  const name =
    (typeof user?.name === 'string' ? user.name : user?.name?.full) || user?.username || '';
  React.useEffect(() => {
    let current = true;
    oxyServices.auth
      .methods()
      .then((methods) => current && setHasPassword(methods.hasPassword))
      .catch(() => current && setHasPassword(false));
    return () => {
      current = false;
    };
  }, [oxyServices]);
  React.useEffect(() => {
    if (hasPassword) onSignedIn({ id: accountId, name });
  }, [hasPassword, accountId, name, onSignedIn]);
  if (hasPassword !== false) return null;
  return <OxyPasswordPanel onDone={() => setHasPassword(true)} />;
}

export function SetupSignIn({ platform, onSignedIn }: SetupSignInProps) {
  const [accountId, setAccountId] = React.useState<string | null>(null);
  const [signedIn, setSignedIn] = React.useState(false);
  const [signingUp, setSigningUp] = React.useState(false);
  const store = React.useMemo(() => handingOverStore(setAccountId), []);
  return (
    <View testID="login-setup-sign-in" style={{ width: '100%', maxWidth: 448 }}>
      <OxyProvider baseURL={platform.api} clientId={platform.clientId} authStateStore={store}>
        {signedIn && accountId ? (
          <AfterSignIn accountId={accountId} onSignedIn={onSignedIn} />
        ) : signingUp ? (
          <OxySignUpPanel onSignedIn={() => setSignedIn(true)} onSignIn={() => setSigningUp(false)} />
        ) : (
          <OxySignInPanel onSignedIn={() => setSignedIn(true)} onCreateAccount={() => setSigningUp(true)} />
        )}
      </OxyProvider>
    </View>
  );
}
