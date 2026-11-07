/**
 * The login screen's setup sign-in (docs/adr/0020-oxy-account-login.mdx, decision 4): the Oxy SDK's
 * own account chooser, the one every Oxy app's account dialog shows (`OxyAuthChooser` from
 * `@oxy.so/services`), bare on the login screen's card (`OxySurfaceFrame`, which the chooser sizes
 * as it sizes the dialog: its split card, with the Commons QR a phone scans, from `md`), inside
 * `OxyProvider` with OxyOS's client id and Oxy's API as the accounts' service names them, and the
 * SDK's seam for a platform's setup client, `authStateStore` (oxy ADR 0058, B3): a store that keeps
 * nothing and hands the credential a sign-in commits to the Bloom host at once
 * (`OxyOSAccountSession.holdCredential`), which holds it until the person gives the account's
 * password once more. Every way in is the SDK's: the email's code or link, the password, the
 * authenticator, the QR approved on a phone, or a new account. An account without a password sets
 * one through the SDK's own flow (`OxyPasswordPanel`) before that. Nothing here is a sign-in of
 * OxyOS's own.
 */
import * as React from 'react';
import { TurboModuleRegistry, View } from 'react-native';
import { OxyAuthChooser, OxyPasswordPanel, OxyProvider, OxySurfaceFrame, useOxy } from '@oxy.so/services';
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

/**
 * The chooser until a sign-in signed the account in, whichever way it came (the chooser's own
 * steps, or the QR the provider's controller completes): the account the host was handed the
 * credential of, now the provider's. Then the password step.
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
