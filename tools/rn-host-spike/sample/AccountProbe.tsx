/**
 * The machine's Oxy session as an application's Oxy SDK finds it (docs/adr/0020-oxy-account-login.mdx,
 * decision 6; oxyos/session-host.js): the session host the runtime's framework registered, read where
 * the SDK reads it (`globalThis[Symbol.for('oxy.so/session-host')]`), its snapshot followed as the
 * SDK follows it, a token asked for, and the SDK's requests (managing the account, signing in
 * again). What the token is for is shown (its `azp`, the application it was minted for), never the
 * token. scenarios/account-session*.json run it against a played
 * accounts' service.
 */
import * as React from 'react';
import { Pressable, Text, View } from 'react-native';

type Snapshot = {
  status: 'signed_in' | 'signed_out' | 'unavailable';
  revision: number;
  account: { id: string; handle: string; displayName?: string; avatar?: string } | null;
  online: boolean;
};
type SessionHost = {
  contract: number;
  platform: string;
  snapshot(): Snapshot;
  subscribe(listener: () => void): () => void;
  getAccessToken(request: { reason: string }): Promise<{ accessToken: string; expiresAt: string } | null>;
  request(action: 'sign_in' | 'switch_account' | 'sign_out' | 'manage_account'): Promise<void>;
};

const host = (globalThis as Record<symbol, unknown>)[Symbol.for('oxy.so/session-host')] as SessionHost | undefined;

/** Whom a token names: its claims' `azp` (a JWT's middle part, or a played token's after "played."). */
function audience(token: string): string {
  const parts = token.split('.');
  const claims = parts.length === 3 ? parts[1]! : parts[1] ?? '';
  try {
    const padded = claims.replace(/-/g, '+').replace(/_/g, '/');
    return JSON.parse(globalThis.atob(padded + '='.repeat((4 - (padded.length % 4)) % 4))).azp ?? 'nobody';
  } catch {
    return 'an unreadable token';
  }
}

function Row({ testID, text }: { testID: string; text: string }) {
  return (
    <Text testID={testID} style={{ fontSize: 16, marginBottom: 8, color: '#111' }}>
      {text}
    </Text>
  );
}

export default function AccountProbe() {
  const snapshot = React.useSyncExternalStore(
    (listener) => host?.subscribe(listener) ?? (() => {}),
    () => host?.snapshot() ?? null,
  );
  const [token, setToken] = React.useState('Token not asked');
  const ask = (reason: string) => {
    if (!host) return;
    host.getAccessToken({ reason }).then(
      (given) => setToken(given ? `Token for ${audience(given.accessToken)} until ${given.expiresAt}` : 'No token'),
      (error: Error) => setToken(`Token refused: ${error.message}`),
    );
  };
  const [requested, setRequested] = React.useState('Nothing requested');
  const request = (action: 'sign_in' | 'manage_account') => {
    if (!host) return;
    host.request(action).then(
      () => setRequested(`Requested ${action}`),
      (error: Error) => setRequested(`Request ${action} failed: ${error.message}`),
    );
  };
  const account = snapshot?.account;
  return (
    <View style={{ flex: 1, padding: 24, backgroundColor: '#fff' }}>
      <Row testID="account-host" text={host ? `Host ${host.platform}, contract ${host.contract}` : 'No session host'} />
      {snapshot && (
        <>
          <Row testID="account-status" text={`Status ${snapshot.status}, ${snapshot.online ? 'online' : 'offline'}`} />
          <Row
            testID="account-who"
            text={account ? `Signed in as ${account.displayName ?? account.handle} (@${account.handle})` : 'No account'}
          />
        </>
      )}
      <Row testID="account-token-said" text={token} />
      <Pressable
        testID="account-token"
        accessibilityRole="button"
        onPress={() => ask('boot')}
        style={{ alignSelf: 'flex-start', paddingHorizontal: 16, paddingVertical: 8, backgroundColor: '#ddd' }}
      >
        <Text style={{ fontSize: 16, color: '#111' }}>Ask for a token</Text>
      </Pressable>
      <Row testID="account-requested" text={requested} />
      {(['manage_account', 'sign_in'] as const).map((action) => (
        <Pressable
          key={action}
          accessibilityRole="button"
          onPress={() => request(action)}
          style={{ alignSelf: 'flex-start', marginTop: 8, paddingHorizontal: 16, paddingVertical: 8, backgroundColor: '#ddd' }}
        >
          <Text style={{ fontSize: 16, color: '#111' }}>{action === 'sign_in' ? 'Sign in again' : 'Manage account'}</Text>
        </Pressable>
      ))}
    </View>
  );
}
