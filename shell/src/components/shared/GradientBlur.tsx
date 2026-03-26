interface GradientBlurProps {
  direction: "top" | "bottom" | "left" | "right";
  size?: number;
}

const maskSteps = [
  { blur: 0.5, from: 0, mid1: 12.5, mid2: 25, to: 37.5 },
  { blur: 1, from: 12.5, mid1: 25, mid2: 37.5, to: 50 },
  { blur: 2, from: 25, mid1: 37.5, mid2: 50, to: 62.5 },
  { blur: 4, from: 37.5, mid1: 50, mid2: 62.5, to: 75 },
  { blur: 8, from: 50, mid1: 62.5, mid2: 75, to: 87.5 },
  { blur: 16, from: 62.5, mid1: 75, mid2: 87.5, to: 100 },
  { blur: 32, from: 75, mid1: 87.5, mid2: 100, to: 100 },
  { blur: 64, from: 87.5, mid1: 100, mid2: 100, to: 100 },
];

function makeMask(dir: string, from: number, mid1: number, mid2: number, to: number) {
  const mask = `linear-gradient(${dir}, rgba(0,0,0,0) ${from}%, rgba(0,0,0,1) ${mid1}%, rgba(0,0,0,1) ${mid2}%, rgba(0,0,0,0) ${to}%)`;
  return { mask, WebkitMask: mask } as const;
}

export default function GradientBlur({ direction, size = 60 }: GradientBlurProps) {
  const isVertical = direction === "top" || direction === "bottom";
  const maskDir = `to ${direction}`;

  const posStyle: React.CSSProperties = {
    position: "absolute",
    pointerEvents: "none",
    zIndex: 20,
    ...(direction === "top" && { top: 0, left: 0, right: 0, height: size }),
    ...(direction === "bottom" && { bottom: 0, left: 0, right: 0, height: size }),
    ...(direction === "left" && { top: 0, bottom: 0, left: 0, width: size }),
    ...(direction === "right" && { top: 0, bottom: 0, right: 0, width: size }),
  };

  return (
    <div style={posStyle}>
      {maskSteps.map((step, i) => (
        <div
          key={i}
          style={{
            position: "absolute",
            inset: 0,
            zIndex: 20 + i,
            backdropFilter: `blur(${step.blur}px)`,
            WebkitBackdropFilter: `blur(${step.blur}px)`,
            ...makeMask(maskDir, step.from, step.mid1, step.mid2, step.to),
          }}
        />
      ))}
    </div>
  );
}
