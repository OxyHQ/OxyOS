import { AliaChatContent } from '@alia.onl/sdk/text';
import { useOxy } from '@oxy.so/services/ui/client';
import { motion } from 'framer-motion';
import { useAliaStore } from '../../stores/aliaStore';

/** OxyOS owns the window; the shared SDK owns the conversation and account-bound state. */
export default function AliaPanel() {
  const close = useAliaStore((state) => state.close);
  const { openAccountDialog, activeSessionId, user } = useOxy();
  return (
    <motion.section
      initial={{ opacity: 0, y: -8, scale: 0.97 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: -8, scale: 0.97 }}
      aria-label="Alia"
      className="liquid-glass fixed top-4 right-4 z-50 flex w-[420px] flex-col overflow-hidden rounded-[20px] border border-white/20"
      style={{ height: 'calc(100vh - 32px)' }}
    >
      <header className="flex items-center justify-between px-4 py-3">
        <span className="text-sm font-semibold text-white/90">Alia</span>
        <div className="flex gap-3">
          <button
            type="button"
            onClick={() => openAccountDialog()}
            className="text-sm text-white/70"
          >
            Account
          </button>
          <button
            type="button"
            onClick={close}
            aria-label="Close Alia"
            className="text-sm text-white/70"
          >
            Close
          </button>
        </div>
      </header>
      <AliaChatContent
        key={`${activeSessionId ?? 'signed-out'}:${user?.id ?? ''}`}
        app="oxyos"
        model="auto"
        voice={false}
        isDarkMode
        layout="sheet"
        clientContext="You are Alia, the assistant in the OxyOS desktop shell."
        onSignInRequired={() => openAccountDialog('signin')}
      />
    </motion.section>
  );
}
