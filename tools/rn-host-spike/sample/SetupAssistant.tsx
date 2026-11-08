/**
 * The setup assistant's frame on OxyOS (docs/adr/0020-oxy-account-login.mdx, decision 7): the Oxy
 * SDK's own (`OxySetupFlow` from `@oxy.so/services`, oxy ADR 0064), on the login screen's card,
 * with the SDK's own sign-in as its sign-in page (`oxySignInStep`: `OxyAuthChooser`, the chooser
 * every Oxy app's account dialog shows, its split card with the Commons QR a phone scans from
 * `md`), inside `OxyProvider` with OxyOS's client id and Oxy's API as the accounts' service names
 * them. The pages OxyOS adds (apps/login/src/SetupAssistant.tsx) are the flow's steps: one frame,
 * one header, one Back and Continue.
 *
 * The SDK's seam for a platform's setup client, `authStateStore` (oxy ADR 0058, B3), keeps nothing
 * and hands the credential a sign-in commits to the Bloom host at once
 * (`OxyOSAccountSession.holdCredential`), which holds it until the setup ends. It also tells how
 * the account signed in (`save`'s context, @oxy.so/services 14.15.2): the QR (a Commons approval:
 * the PIN alone, never a password), the password (typed here: taken as it was typed, held only
 * until the setup uses it, never asked again), or another way.
 *
 * The QR and the password go on at once: nothing more needs Oxy before the PIN. Waiting here on
 * the SDK's own work after a sign-in (its device's set, the account's profile) put a stalled
 * network between the approval and the next page, and a read that timed out became an error
 * (2026-11-08). Another way (the email's code, a new account) needs to know whether the account
 * has a password: its sign-in methods are read, tried again by themselves while the network
 * stalls, and only then said unread, with why and what to do; never guessed (failing closed).
 */
import * as React from 'react';
import { ScrollView, TurboModuleRegistry, View, useWindowDimensions } from 'react-native';
import { OxyProvider, OxySetupFlow, oxySignInStep, useOxy, type OxySetupStep } from '@oxy.so/services';
import { Loading } from '@oxy.so/bloom/loading';
import { Text } from '@oxy.so/bloom/typography';
import { useTheme } from '@oxy.so/bloom/theme';
import type { AssistantPage, SetupAssistantFrameProps, SignedIn } from '../../../apps/login/src/SetupAssistant';
import type { SignedInAccount } from '../../../apps/login/src/setup';
import { useLoginMessages } from '../../../apps/login/src/useLoginMessages';

const native = TurboModuleRegistry.get<{
  holdCredential(deviceId: string, deviceSecret: string, accountId: string): boolean;
}>('OxyOSAccountSession');

type Persisted = { sessionId: string; userId: string; deviceId?: string; deviceSecret?: string };
/** What the SDK tells a platform's setup store of the sign-in it commits (14.15.2). */
type SaveContext = { method?: unknown; password?: unknown };
/** How the account signed in, and the password typed for a password sign-in. */
type Handed = { accountId: string; via: SignedInAccount['via']; password?: string };

/** How the SDK says the account signed in, read strictly: anything else is another way. */
function viaOf(context: SaveContext | undefined): { via: SignedInAccount['via']; password?: string } {
  if (context?.method === 'qr') return { via: 'qr' };
  if (context?.method === 'password' && typeof context.password === 'string' && context.password) {
    return { via: 'password', password: context.password };
  }
  return { via: 'other' };
}

/** The setup's store: nothing kept, nothing loaded; a committed credential goes to the host. */
function handingOverStore(onHanded: (handed: Handed) => void) {
  return {
    load: async () => null,
    save: async (state: Persisted, context?: SaveContext) => {
      if (!state.deviceId || !state.deviceSecret || !state.userId || !native) return false;
      const held = native.holdCredential(state.deviceId, state.deviceSecret, state.userId);
      if (held) {
        const handed = { accountId: state.userId, ...viaOf(context) };
        // How it signed in, for the log; never the password.
        console.info(`[setup] signed in by ${handed.via}${handed.password ? ', its password typed' : ''}`);
        onHanded(handed);
      }
      return held;
    },
    clear: async () => {},
  };
}

/** Pauses between reads of the sign-in methods while the network stalls, before saying so. */
const METHODS_RETRY_MS = [2000, 4000, 8000];

/** The account's sign-in methods, for a sign-in by another way: read, or why not yet. */
function useSignInMethods(handed: Handed | null, signedIn: SignedIn) {
  const { oxyServices, user } = useOxy() as {
    oxyServices: { auth: { methods(): Promise<{ hasPassword?: unknown }> } };
    user?: { id?: string; username?: string; name?: { full?: string } | string } | null;
  };
  const [unread, setUnread] = React.useState(false);
  const [attempt, setAttempt] = React.useState(0);
  const ready = handed?.via === 'other' && user?.id === handed.accountId;
  const name = (typeof user?.name === 'string' ? user.name : user?.name?.full) || user?.username || '';
  React.useEffect(() => {
    if (!ready || !handed) return undefined;
    let current = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    setUnread(false);
    const read = (tried: number) => {
      oxyServices.auth
        .methods()
        .then((methods) => {
          if (!current) return;
          if (typeof methods?.hasPassword !== 'boolean') throw new Error('an answer outside the contract');
          signedIn({ id: handed.accountId, name, hasPassword: methods.hasPassword, via: 'other' });
        })
        .catch(() => {
          if (!current) return;
          if (tried < METHODS_RETRY_MS.length) timer = setTimeout(() => read(tried + 1), METHODS_RETRY_MS[tried]);
          else setUnread(true);
        });
    };
    read(0);
    return () => {
      current = false;
      if (timer) clearTimeout(timer);
    };
  }, [ready, handed, attempt]);
  return { unread, again: () => setAttempt((count) => count + 1) };
}

/** The flow, inside the provider: the sign-in page the SDK's chooser until a sign-in handed its
    credential over, then (for another way) its methods being read. */
function Flow({
  pages,
  current,
  onBack,
  backLabel,
  label,
  locale,
  handed,
  onSignedIn,
  onFrameWidth,
}: Omit<SetupAssistantFrameProps, 'platform'> & {
  handed: Handed | null;
  onFrameWidth: (width: number | null) => void;
}) {
  const words = useLoginMessages().setup;
  const theme = useTheme();
  const { unread, again } = useSignInMethods(handed, onSignedIn);
  const steps = pages.map((page: AssistantPage): OxySetupStep => {
    if (!page.signIn) return stepOf(page);
    if (!handed) return oxySignInStep({ id: page.id });
    // Another way's account, its methods being read (the QR and the password went on at once).
    return {
      id: page.id,
      title: words.title,
      content: (
        <View style={{ alignItems: 'center', gap: 16, paddingVertical: 16 }}>
          {unread ? null : <Loading size="lg" />}
          <Text
            testID="login-setup-methods"
            accessibilityLiveRegion="polite"
            style={{ fontSize: 15, lineHeight: 22, textAlign: 'center', color: unread ? theme.colors.text : theme.colors.textSecondary }}
          >
            {unread ? words.accountUnchecked : words.checkingAccount}
          </Text>
        </View>
      ),
      primary: unread ? { label: words.retry, onPress: again, testID: 'login-setup-methods-retry' } : undefined,
    };
  });
  return (
    <OxySetupFlow
      label={label}
      steps={steps}
      current={current}
      onBack={onBack}
      backLabel={backLabel}
      locale={locale}
      onMaxWidthChange={onFrameWidth}
    />
  );
}

/** A host's page as the SDK's flow takes it (the sign-in page is the provider's). */
function stepOf(page: AssistantPage): OxySetupStep {
  const { signIn: _signIn, ...step } = page;
  return step;
}

/** The card the flow is on, over the dimmed wallpaper, as a Mac's setup shows its pages: as
    wide as the page asks (the sign-in's split card, with the QR), else the flow's own. */
export function SetupAssistantFrame(props: SetupAssistantFrameProps) {
  const { platform, onSignedIn, pages, current } = props;
  const { height } = useWindowDimensions();
  const [width, setWidth] = React.useState<number | null>(null);
  // Held while the next page is chosen; the password, once handed on, is the login screen's to
  // use and forget.
  const [handed, setHanded] = React.useState<Handed | null>(null);
  // A new sign-in each time the chooser comes back (the last one given up): a provider of its own,
  // so nothing of the one before is shown signed in.
  const chooser = pages.some((page) => page.id === current && page.signIn);
  const [attempt, setAttempt] = React.useState(0);
  const shown = React.useRef(chooser);
  React.useEffect(() => {
    if (chooser && !shown.current) {
      setHanded(null);
      setAttempt((count) => count + 1);
    }
    shown.current = chooser;
  }, [chooser]);
  const store = React.useMemo(
    () =>
      handingOverStore((given) => {
        setHanded(given);
        // The QR and the password need nothing more of Oxy: the next page at once.
        if (given.via !== 'other') onSignedIn({ id: given.accountId, name: '', hasPassword: given.via === 'password', via: given.via }, given.password);
      }),
    [attempt],
  );
  return (
    <View
      testID="login-setup-card"
      style={{
        width: width ?? 560,
        maxWidth: '100%',
        backgroundColor: 'rgba(18,18,22,0.88)',
        borderRadius: 24,
        overflow: 'hidden',
      }}
    >
      {/* Taller than the screen (a code under a form), it scrolls within its card. */}
      <ScrollView
        style={{ maxHeight: Math.max(240, height - 96) }}
        contentContainerStyle={{ paddingHorizontal: 32, paddingVertical: 28 }}
      >
        {platform ? (
          <OxyProvider key={attempt} baseURL={platform.api} clientId={platform.clientId} authStateStore={store}>
            <Flow {...props} handed={handed} onFrameWidth={setWidth} />
          </OxyProvider>
        ) : (
          // Nothing to sign in to (offline before the platform is known, or no client id): the
          // flow's own pages, no SDK provider.
          <OxySetupFlow
            label={props.label}
            steps={pages.map(stepOf)}
            current={current}
            onBack={props.onBack}
            backLabel={props.backLabel}
            locale={props.locale}
            onMaxWidthChange={setWidth}
          />
        )}
      </ScrollView>
    </View>
  );
}
