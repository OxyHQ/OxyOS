import { useState, useEffect, useCallback, useRef } from "react";
import { useOxGlass } from "../../hooks/useOxGlass";
import OxGlassFilter from "./OxGlassFilter";

interface OxGlassSwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  size?: "xs" | "small" | "medium";
  disabled?: boolean;
}

const sizePresets = {
  xs: { sliderWidth: 70, sliderHeight: 30, thumbWidth: 64, thumbHeight: 40, thumbScale: 0.65, bezel: 8, glassThickness: 10 },
  small: { sliderWidth: 100, sliderHeight: 42, thumbWidth: 92, thumbHeight: 58, thumbScale: 0.65, bezel: 14, glassThickness: 15 },
  medium: { sliderWidth: 130, sliderHeight: 54, thumbWidth: 119, thumbHeight: 75, thumbScale: 0.65, bezel: 16, glassThickness: 20 },
};

const THUMB_REST_SCALE = 0.65;
const THUMB_ACTIVE_SCALE = 0.9;

export default function OxGlassSwitch({
  checked,
  onChange,
  size = "xs",
  disabled = false,
}: OxGlassSwitchProps) {
  const dims = sizePresets[size];
  const [isPressed, setIsPressed] = useState(false);
  const [dragRatio, setDragRatio] = useState(checked ? 1 : 0);
  const initialXRef = useRef(0);

  useEffect(() => {
    if (!isPressed) setDragRatio(checked ? 1 : 0);
  }, [checked, isPressed]);

  const thumbRadius = dims.thumbHeight / 2;
  const restOffset = ((1 - THUMB_REST_SCALE) * dims.thumbWidth) / 2;
  const travel = dims.sliderWidth - dims.sliderHeight - (dims.thumbWidth - dims.thumbHeight) * THUMB_REST_SCALE;

  const thumbScale = isPressed ? THUMB_ACTIVE_SCALE : THUMB_REST_SCALE;

  const thumbGlass = useOxGlass({
    surface: "lip",
    shape: "pill",
    bezel: dims.bezel,
    glassThickness: dims.glassThickness,
    refraction: 1.5,
    blur: 0.2,
    scaleRatio: isPressed ? 0.9 : 0.4,
    specularOpacity: 0.5,
    specularSaturation: 6,
    bgOpacity: isPressed ? 0.1 : 1,
  });
  const thumbX = dragRatio * travel;
  const thumbMarginLeft = -restOffset + (dims.sliderHeight - dims.thumbHeight * THUMB_REST_SCALE) / 2;

  // Background color interpolation
  const r = Math.round(148 + (59 - 148) * dragRatio);
  const g = Math.round(148 + (191 - 148) * dragRatio);
  const b = Math.round(159 + (78 - 159) * dragRatio);
  const a = Math.round(119 + (238 - 119) * dragRatio);
  const bgColor = `rgba(${r}, ${g}, ${b}, ${a / 255})`;

  const handlePointerDown = useCallback((e: React.PointerEvent) => {
    if (disabled) return;
    setIsPressed(true);
    initialXRef.current = e.clientX;
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
  }, [disabled]);

  const handlePointerMove = useCallback((e: React.PointerEvent) => {
    if (!isPressed || disabled) return;
    const baseRatio = checked ? 1 : 0;
    const displacement = e.clientX - initialXRef.current;
    const ratio = baseRatio + displacement / travel;
    const overflow = ratio < 0 ? -ratio : ratio > 1 ? ratio - 1 : 0;
    const overflowSign = ratio < 0 ? -1 : 1;
    const dampedOverflow = (overflowSign * overflow) / 22;
    setDragRatio(Math.min(1, Math.max(0, ratio)) + dampedOverflow);
  }, [isPressed, disabled, checked, travel]);

  const handlePointerUp = useCallback((e: React.PointerEvent) => {
    if (!isPressed) return;
    setIsPressed(false);
    const distance = e.clientX - initialXRef.current;
    if (Math.abs(distance) > 4) {
      const shouldBeChecked = dragRatio > 0.5;
      setDragRatio(shouldBeChecked ? 1 : 0);
      onChange(shouldBeChecked);
    } else {
      const newValue = !checked;
      setDragRatio(newValue ? 1 : 0);
      onChange(newValue);
    }
  }, [isPressed, dragRatio, checked, onChange]);

  useEffect(() => {
    if (!isPressed) return;
    const up = () => {
      setIsPressed(false);
      const shouldBeChecked = dragRatio > 0.5;
      setDragRatio(shouldBeChecked ? 1 : 0);
      onChange(shouldBeChecked);
    };
    window.addEventListener("pointerup", up);
    return () => window.removeEventListener("pointerup", up);
  }, [isPressed, dragRatio, onChange]);

  return (
    <div
      className={`inline-block touch-none select-none ${disabled ? "cursor-not-allowed opacity-50" : ""}`}
      onPointerMove={handlePointerMove}
    >
      <div
        className="relative cursor-pointer transition-colors duration-150"
        style={{
          width: dims.sliderWidth,
          height: dims.sliderHeight,
          backgroundColor: bgColor,
          borderRadius: dims.sliderHeight / 2,
        }}
      >
        {/* Thumb — glass applied directly */}
        {thumbGlass.filterData && (
          <OxGlassFilter
            filterId={thumbGlass.filterId}
            filterData={thumbGlass.filterData}
            blur={thumbGlass.blur}
            specularOpacity={thumbGlass.specularOpacity}
            specularSaturation={thumbGlass.specularSaturation}
          />
        )}
        <div
          ref={thumbGlass.ref}
          className="absolute cursor-pointer transition-transform duration-100 ease-out"
          style={{
            ...thumbGlass.glassStyle,
            height: dims.thumbHeight,
            width: dims.thumbWidth,
            marginLeft: thumbMarginLeft,
            transform: `translateX(${thumbX}px) translateY(-50%) scale(${thumbScale})`,
            top: dims.sliderHeight / 2,
            borderRadius: thumbRadius,
            boxShadow: isPressed
              ? "0 4px 22px rgba(0,0,0,0.1), inset 2px 7px 24px rgba(0,0,0,0.09), inset -2px -7px 24px rgba(255,255,255,0.09)"
              : "0 4px 22px rgba(0,0,0,0.1)",
          }}
          onPointerDown={handlePointerDown}
          onPointerUp={handlePointerUp}
        />
      </div>
    </div>
  );
}
