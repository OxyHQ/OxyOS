import { motion } from 'framer-motion';
import { useAliaStore } from '../../stores/aliaStore';
import { AliaAvatar } from '@alia.onl/sdk/avatar';

export default function AliaBubble() {
  const isOpen = useAliaStore((s) => s.isOpen);
  const toggle = useAliaStore((s) => s.toggle);

  if (isOpen) return null;

  return (
    <motion.button
      initial={{ opacity: 0, scale: 0.8 }}
      animate={{ opacity: 1, scale: 1 }}
      exit={{ opacity: 0, scale: 0.8 }}
      transition={{ duration: 0.2, ease: [0.2, 0, 0, 1] }}
      onClick={toggle}
      className="fixed top-4 right-4 z-50 flex h-11 w-11 cursor-pointer items-center justify-center rounded-full border border-white/20 shadow-[0_4px_20px_rgba(0,0,0,0.3),inset_0_0.5px_0_rgba(255,255,255,0.12)] liquid-glass transition-transform duration-150 hover:scale-110"
      aria-label="Open Alia"
    >
      <AliaAvatar size={28} />
    </motion.button>
  );
}
