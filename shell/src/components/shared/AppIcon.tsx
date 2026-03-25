import { motion } from "framer-motion";

interface AppIconProps {
  name: string;
  color: string;
  icon: React.ReactNode;
  size?: number;
  showLabel?: boolean;
  onClick?: () => void;
}

export default function AppIcon({
  name,
  color,
  icon,
  size = 44,
  showLabel = false,
  onClick,
}: AppIconProps) {
  return (
    <motion.button
      type="button"
      className="flex cursor-pointer flex-col items-center gap-1.5 border-none bg-transparent p-0 transition-transform duration-100 active:scale-90"
      whileTap={{ scale: 0.9 }}
      onClick={onClick}
      aria-label={name}
    >
      <div
        className="flex items-center justify-center rounded-full shadow-[0_1px_4px_rgba(0,0,0,0.2)]"
        style={{ backgroundColor: color, width: size, height: size }}
      >
        {icon}
      </div>
      {showLabel && (
        <span className="max-w-[72px] truncate text-center text-[11px] font-medium leading-tight text-white/80 drop-shadow-[0_1px_2px_rgba(0,0,0,0.5)]">
          {name}
        </span>
      )}
    </motion.button>
  );
}
