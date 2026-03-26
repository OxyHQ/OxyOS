import { useRef, useState, useEffect, useCallback, useId } from "react";
import OxGlass from "./OxGlass";

interface OxGlassSliderProps {
  value: number;
  onChange: (value: number) => void;
  min?: number;
  max?: number;
  size?: "small" | "medium" | "large";
  disabled?: boolean;
}

const sizePresets = {
  small: { sliderHeight: 10, thumbWidth: 54, thumbHeight: 36, bezel: 30, glassThickness: 30 },
  medium: { sliderHeight: 14, thumbWidth: 90, thumbHeight: 60, bezel: 40, glassThickness: 40 },
  large: { sliderHeight: 18, thumbWidth: 126, thumbHeight: 84, bezel: 50, glassThickness: 40 },
};

const SCALE_REST = 0.6;
const SCALE_DRAG = 1;

export default function OxGlassSlider({
  value,
  onChange,
  min = 0,
  max = 100,
  size = "small",
  disabled = false,
}: OxGlassSliderProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [containerWidth, setContainerWidth] = useState(300);
  const [isDragging, setIsDragging] = useState(false);
  const startXRef = useRef(0);
  const startThumbXRef = useRef(0);

  const dims = sizePresets[size];
  const normalized = (value - min) / (max - min);
  const thumbX = normalized * (containerWidth - dims.thumbWidth);

  const scale = isDragging ? SCALE_DRAG : SCALE_REST;
  const bgOpacity = isDragging ? 0.1 : 1;
  const scaleRatio = isDragging ? 0.9 : 0.4;

  // Track container width
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width;
      if (w) setContainerWidth(Math.round(w));
    });
    ro.observe(el);
    setContainerWidth(el.offsetWidth);
    return () => ro.disconnect();
  }, []);

  const handlePointerDown = useCallback((e: React.PointerEvent) => {
    if (disabled) return;
    setIsDragging(true);
    startXRef.current = e.clientX;
    startThumbXRef.current = thumbX;
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
  }, [disabled, thumbX]);

  const handlePointerMove = useCallback((e: React.PointerEvent) => {
    if (!isDragging || disabled) return;
    const deltaX = e.clientX - startXRef.current;
    const maxThumbX = containerWidth - dims.thumbWidth;
    const newThumbX = Math.max(0, Math.min(maxThumbX, startThumbXRef.current + deltaX));
    const normalizedValue = maxThumbX > 0 ? newThumbX / maxThumbX : 0;
    onChange(min + normalizedValue * (max - min));
  }, [isDragging, disabled, containerWidth, dims.thumbWidth, min, max, onChange]);

  const handlePointerUp = useCallback(() => {
    setIsDragging(false);
  }, []);

  useEffect(() => {
    if (!isDragging) return;
    const up = () => setIsDragging(false);
    window.addEventListener("pointerup", up);
    return () => window.removeEventListener("pointerup", up);
  }, [isDragging]);

  return (
    <div
      ref={containerRef}
      className={`relative w-full ${disabled ? "cursor-not-allowed opacity-50" : ""}`}
      style={{ height: dims.thumbHeight }}
    >
      {/* Track */}
      <div
        className={`absolute ${disabled ? "cursor-not-allowed" : "cursor-pointer"}`}
        style={{
          height: dims.sliderHeight,
          top: (dims.thumbHeight - dims.sliderHeight) / 2,
          left: (dims.thumbWidth * (1 - SCALE_REST)) / 2,
          right: (dims.thumbWidth * (1 - SCALE_REST)) / 2,
          backgroundColor: "#89898F66",
          borderRadius: dims.sliderHeight / 2,
        }}
      >
        <div className="h-full w-full overflow-hidden rounded-full">
          {/* Blue fill */}
          <div
            style={{
              height: dims.sliderHeight,
              width: Math.max(0, thumbX + dims.thumbWidth / 2 - (dims.thumbWidth * (1 - SCALE_REST)) / 2),
              borderRadius: dims.sliderHeight / 2,
              backgroundColor: "#0377F7",
            }}
          />
        </div>
      </div>

      {/* Thumb */}
      <OxGlass
        surface="convex"
        shape="pill"
        bezel={dims.bezel}
        glassThickness={dims.glassThickness}
        refraction={1.45}
        blur={0}
        scaleRatio={scaleRatio}
        specularOpacity={0.4}
        specularSaturation={7}
        bgOpacity={bgOpacity}
        className={`absolute transition-transform duration-150 ease-out ${disabled ? "cursor-not-allowed" : "cursor-pointer"}`}
        style={{
          height: dims.thumbHeight,
          width: dims.thumbWidth,
          top: "50%",
          left: thumbX + dims.thumbWidth / 2,
          borderRadius: dims.thumbHeight / 2,
          transform: `translate(-50%, -50%) scale(${scale})`,
          transformOrigin: "center center",
          boxShadow: "0 3px 14px rgba(0,0,0,0.1)",
        }}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
      >
        <div style={{ width: "100%", height: "100%" }} />
      </OxGlass>
    </div>
  );
}
