import { motion } from "framer-motion";

export type AliaExpression = "idle" | "thinking" | "speaking" | "listening";

interface AliaFaceProps {
  size?: number;
  expression?: AliaExpression;
}

export default function AliaFace({ size = 28, expression = "idle" }: AliaFaceProps) {
  const r = size / 2;
  const eyeY = r * 0.42;
  const eyeSpacing = r * 0.32;
  const eyeRx = r * 0.09;
  const eyeRy = expression === "thinking" ? r * 0.04 : r * 0.1;

  return (
    <motion.svg
      width={size}
      height={size}
      viewBox={`0 0 ${size} ${size}`}
      animate={
        expression === "speaking"
          ? { scale: [1, 1.04, 1] }
          : expression === "listening"
            ? { scale: [1, 1.06, 1] }
            : undefined
      }
      transition={
        expression === "speaking" || expression === "listening"
          ? { duration: expression === "speaking" ? 0.6 : 1.2, repeat: Infinity, ease: "easeInOut" }
          : undefined
      }
    >
      {/* Face circle — gradient */}
      <defs>
        <linearGradient id="alia-grad" x1="0%" y1="0%" x2="100%" y2="100%">
          <stop offset="0%" stopColor="#0a84ff" stopOpacity="0.5" />
          <stop offset="100%" stopColor="#bf5af2" stopOpacity="0.5" />
        </linearGradient>
      </defs>
      <circle cx={r} cy={r} r={r - 0.5} fill="url(#alia-grad)" stroke="white" strokeOpacity="0.15" strokeWidth="0.5" />

      {/* Eyes */}
      <motion.ellipse
        cx={r - eyeSpacing}
        cy={eyeY}
        rx={eyeRx}
        ry={eyeRy}
        fill="white"
        fillOpacity="0.9"
        animate={
          expression === "thinking"
            ? { cy: [eyeY, eyeY - 1, eyeY] }
            : expression === "listening"
              ? { ry: [eyeRy, eyeRy * 1.4, eyeRy] }
              : undefined
        }
        transition={{ duration: 1.5, repeat: Infinity, ease: "easeInOut" }}
      />
      <motion.ellipse
        cx={r + eyeSpacing}
        cy={eyeY}
        rx={eyeRx}
        ry={eyeRy}
        fill="white"
        fillOpacity="0.9"
        animate={
          expression === "thinking"
            ? { cy: [eyeY, eyeY - 1, eyeY] }
            : expression === "listening"
              ? { ry: [eyeRy, eyeRy * 1.4, eyeRy] }
              : undefined
        }
        transition={{ duration: 1.5, repeat: Infinity, ease: "easeInOut", delay: 0.1 }}
      />

      {/* Mouth — changes by expression */}
      {expression === "speaking" ? (
        <motion.ellipse
          cx={r}
          cy={r * 1.2}
          rx={r * 0.15}
          ry={r * 0.08}
          fill="white"
          fillOpacity="0.6"
          animate={{ ry: [r * 0.06, r * 0.12, r * 0.06] }}
          transition={{ duration: 0.4, repeat: Infinity, ease: "easeInOut" }}
        />
      ) : expression === "listening" ? (
        <circle cx={r} cy={r * 1.2} r={r * 0.06} fill="white" fillOpacity="0.5" />
      ) : (
        <path
          d={`M ${r - r * 0.18} ${r * 1.15} Q ${r} ${r * 1.3} ${r + r * 0.18} ${r * 1.15}`}
          fill="none"
          stroke="white"
          strokeOpacity="0.5"
          strokeWidth="0.8"
          strokeLinecap="round"
        />
      )}
    </motion.svg>
  );
}
