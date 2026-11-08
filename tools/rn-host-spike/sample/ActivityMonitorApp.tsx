/**
 * OxyOS's Activity Monitor (apps/activity-monitor, docs/adr/0060-activity-monitor.mdx) in the Bloom
 * host. Its source is the platform API's `activity` (`@oxyos/platform`, docs/adr/0058-the-platform-api.mdx),
 * which only the client the session granted `activity` (`so.oxy.activitymonitor`) may use: the
 * samples it follows while mounted (the session samples only while its window is shown), the view
 * it samples, quitting the person's own processes and apps, and inspecting a process. A call the
 * platform refuses rejects with a PlatformError, which the app reads as the session's answer. Without
 * the host's module (a host run outside a session) there is no source, and the window says it
 * cannot read the computer's activity. Its language is the session's locale
 * (docs/adr/0040-language-and-region.mdx, decision 5), given to `BloomProvider` and followed live.
 */
import * as React from 'react';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { activity, isPlatformError } from '@oxyos/platform';
import { oxyosSurfaces } from './oxyos/platform-surface';
import { useSessionLocale } from './oxyos/locale';
import { ActivityMonitor } from '../../../apps/activity-monitor/src/ActivityMonitor';
import type { ActivitySource, SessionAnswer } from '../../../apps/activity-monitor/src/model';

/** A call's answer, or its rejection as the session's `{failed: {reason}}`. */
async function answered(call: Promise<unknown>): Promise<SessionAnswer> {
  try {
    return (await call) as SessionAnswer;
  } catch (error) {
    if (isPlatformError(error)) return { failed: { reason: error.reason ?? error.code } };
    return { failed: { reason: 'failed' } };
  }
}

/** The platform's activity as the app's source: following sets the view, then subscribes. */
const source: ActivitySource | null = activity.available
  ? {
      follow(view, listener) {
        // The view is kept before the first follower begins the following, which sends it.
        activity.setView(view);
        return activity.sample.subscribe(listener as (sample: unknown) => void);
      },
      setView: (view) => activity.setView(view),
      quit: (pid, started, force) => answered(activity.quit(pid, started, force)),
      quitApp: (id, force) => answered(activity.quitApp(id, force)),
      inspect: (pid, started) => answered(activity.inspect(pid, started)),
    }
  : null;

export default function ActivityMonitorApp() {
  const locale = useSessionLocale();
  return (
    <SafeAreaProvider>
      <BloomProvider platform={oxyosSurfaces} locale={locale}>
        <ActivityMonitor source={source} locale={locale} />
      </BloomProvider>
    </SafeAreaProvider>
  );
}
