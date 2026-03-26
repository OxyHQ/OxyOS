import { useEffect, useRef } from "react";
import { motion, useMotionValue, useTransform, animate } from "framer-motion";

// ─── Types ──────────────────────────────────────────────────────────────────

export type AliaExpression =
  | "Idle A"
  | "Interesting"
  | "Top Left"
  | "Greeting"
  | "Searching A"
  | "Thinking"
  | "Error"
  | "Writing E"
  | "Searching F"
  | "Looking Down";

interface AliaFaceProps {
  expression?: AliaExpression;
  size?: number;
}

// ─── Expression data (ported from @alia.onl/sdk AliaFace.tsx) ───────────────
//
// leftEye:  [cx, cy, r]
// rightEye: [cx, cy, r]
// leftBrow: [sx, sy, cx, cy, ex, ey]           M sx sy Q cx cy ex ey
// noseBrow: [sx, sy, c1x, c1y, c2x, c2y,       M sx sy C c1x c1y, c2x c2y, ex ey
//            ex, ey, lx1, ly1, lx2, ly2]         L lx1 ly1 L lx2 ly2

interface ExpressionData {
  leftEye: [number, number, number];
  rightEye: [number, number, number];
  leftBrow: [number, number, number, number, number, number];
  noseBrow: [number, number, number, number, number, number, number, number, number, number, number, number];
}

const EXPRESSIONS: Record<AliaExpression, ExpressionData> = {
  "Idle A": {
    leftEye: [123, 118, 11],
    rightEye: [182, 126, 11],
    leftBrow: [91, 90, 127, 53, 154, 85],
    noseBrow: [224, 98, 195, 57, 177, 79, 160, 110, 98, 225, 149, 230],
  },
  Interesting: {
    leftEye: [127, 104, 11],
    rightEye: [191, 107, 11],
    leftBrow: [97, 91, 122, 68, 141, 76],
    noseBrow: [233, 79, 208, 54, 166, 59, 143, 104, 89, 192, 133, 212],
  },
  "Top Left": {
    leftEye: [135, 74, 11],
    rightEye: [181, 101, 11],
    leftBrow: [114, 56, 145, 28, 169, 52],
    noseBrow: [227, 104, 209, 66, 180, 68, 162, 87, 94, 174, 137, 195],
  },
  Greeting: {
    leftEye: [123, 120, 11],
    rightEye: [182, 126, 11],
    leftBrow: [91, 70, 127, 33, 154, 65],
    noseBrow: [224, 75, 195, 30, 177, 79, 160, 110, 98, 225, 149, 230],
  },
  "Searching A": {
    leftEye: [103, 98, 11],
    rightEye: [162, 106, 11],
    leftBrow: [71, 70, 107, 33, 134, 65],
    noseBrow: [204, 78, 175, 37, 157, 59, 140, 90, 78, 205, 129, 210],
  },
  Thinking: {
    leftEye: [123, 118, 11],
    rightEye: [182, 126, 11],
    leftBrow: [91, 85, 127, 48, 154, 80],
    noseBrow: [235, 85, 205, 65, 177, 79, 160, 110, 98, 225, 149, 230],
  },
  Error: {
    leftEye: [110, 240, 11],
    rightEye: [170, 250, 9],
    leftBrow: [80, 210, 110, 200, 140, 220],
    noseBrow: [250, 250, 230, 180, 203, 203, 180, 220, 120, 280, 160, 280],
  },
  "Writing E": {
    leftEye: [148, 118, 11],
    rightEye: [207, 126, 11],
    leftBrow: [116, 90, 152, 53, 179, 85],
    noseBrow: [249, 98, 220, 57, 202, 79, 185, 110, 123, 225, 174, 230],
  },
  "Searching F": {
    leftEye: [143, 98, 11],
    rightEye: [202, 106, 11],
    leftBrow: [111, 70, 147, 33, 174, 65],
    noseBrow: [244, 78, 215, 37, 197, 59, 180, 90, 118, 205, 169, 210],
  },
  "Looking Down": {
    leftEye: [106, 146, 11],
    rightEye: [168, 146, 11],
    leftBrow: [62, 107, 96, 61, 124, 84],
    noseBrow: [203, 90, 182, 65, 140, 85, 133, 130, 119, 238, 159, 222],
  },
};

const DEFAULT_EXPRESSION: AliaExpression = "Idle A";
const MORPH_MS = 600;

function makeBrowD(b: number[]): string {
  return `M ${b[0]} ${b[1]} Q ${b[2]} ${b[3]} ${b[4]} ${b[5]}`;
}
function makeNoseD(n: number[]): string {
  return `M ${n[0]} ${n[1]} C ${n[2]} ${n[3]}, ${n[4]} ${n[5]}, ${n[6]} ${n[7]} L ${n[8]} ${n[9]} L ${n[10]} ${n[11]}`;
}

const FLOAT_ANIMATE = { y: [0, -2, 0, 2, 0] };
const FLOAT_TRANSITION = { duration: 5, repeat: Infinity, ease: "easeInOut" as const };

export default function AliaFace({ expression = DEFAULT_EXPRESSION, size = 120 }: AliaFaceProps) {
  const data = EXPRESSIONS[expression] ?? EXPRESSIONS[DEFAULT_EXPRESSION];

  // Animated motion values for morphing
  const leCx = useMotionValue(data.leftEye[0]);
  const leCy = useMotionValue(data.leftEye[1]);
  const leR = useMotionValue(data.leftEye[2]);
  const reCx = useMotionValue(data.rightEye[0]);
  const reCy = useMotionValue(data.rightEye[1]);
  const reR = useMotionValue(data.rightEye[2]);

  const browD = useMotionValue(makeBrowD(data.leftBrow));
  const noseD = useMotionValue(makeNoseD(data.noseBrow));

  // Blink
  const blinkScale = useMotionValue(1);

  // Derived transforms for eye ry (blink)
  const leRy = useTransform(() => leR.get() * blinkScale.get());
  const reRy = useTransform(() => reR.get() * blinkScale.get());

  // Morph on expression change
  useEffect(() => {
    const t = EXPRESSIONS[expression] ?? EXPRESSIONS[DEFAULT_EXPRESSION];
    const opts = { duration: MORPH_MS / 1000, ease: [0.4, 0, 0.2, 1] as [number, number, number, number] };
    animate(leCx, t.leftEye[0], opts);
    animate(leCy, t.leftEye[1], opts);
    animate(leR, t.leftEye[2], opts);
    animate(reCx, t.rightEye[0], opts);
    animate(reCy, t.rightEye[1], opts);
    animate(reR, t.rightEye[2], opts);

    // Path morph — snap (CSS can't tween SVG d between different structures)
    const timer = setTimeout(() => {
      browD.set(makeBrowD(t.leftBrow));
      noseD.set(makeNoseD(t.noseBrow));
    }, 0);
    return () => clearTimeout(timer);
  }, [expression]);

  // Blink loop
  const blinkRef = useRef<ReturnType<typeof setInterval>>(undefined);
  useEffect(() => {
    const doBlink = () => {
      animate(blinkScale, 0.1, { duration: 0.08 }).then(() =>
        animate(blinkScale, 1, { duration: 0.12 })
      );
    };
    blinkRef.current = setInterval(doBlink, 3500);
    return () => clearInterval(blinkRef.current);
  }, []);

  return (
    <motion.div
      animate={FLOAT_ANIMATE}
      transition={FLOAT_TRANSITION}
      style={{ width: size, height: size }}
    >
      <svg
        width={size}
        height={size}
        viewBox="35 35 250 250"
        style={{ overflow: "visible" }}
      >
        {/* Background circle */}
        <circle
          cx="160"
          cy="160"
          r="125"
          fill="none"
          stroke="white"
          strokeOpacity="0.12"
          strokeWidth="1"
        />

        {/* Left brow */}
        <motion.path
          d={browD}
          stroke="white"
          strokeWidth="15"
          strokeLinecap="round"
          fill="none"
        />

        {/* Left eye */}
        <motion.ellipse
          cx={leCx}
          cy={leCy}
          rx={leR}
          ry={leRy}
          fill="white"
        />

        {/* Right eye */}
        <motion.ellipse
          cx={reCx}
          cy={reCy}
          rx={reR}
          ry={reRy}
          fill="white"
        />

        {/* Nose + right brow */}
        <motion.path
          d={noseD}
          stroke="white"
          strokeWidth="15"
          strokeLinecap="round"
          strokeLinejoin="round"
          fill="none"
        />
      </svg>
    </motion.div>
  );
}
