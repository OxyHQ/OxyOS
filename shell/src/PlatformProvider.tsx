import type { ReactNode } from 'react';
import { BloomThemeProvider } from '@oxy.so/bloom/theme';
import { OxyProvider } from '@oxy.so/services/ui/client';

const clientId =
  import.meta.env.VITE_OXY_CLIENT_ID || 'oxy_dk_ee7af26b21c2ff9e70f59e43dc2359106280665d779b74a8';

/** The registered desktop public client identifies the sign-in integration, not a secret. */
export default function PlatformProvider({ children }: { children: ReactNode }) {
  return (
    <BloomThemeProvider mode="dark">
      <OxyProvider clientId={clientId} baseURL="https://api.oxy.so">
        {children}
      </OxyProvider>
    </BloomThemeProvider>
  );
}
