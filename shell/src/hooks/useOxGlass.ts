import { useId, useRef, useState, useEffect, useMemo } from "react";
import {
  SURFACES,
  calculateDisplacementProfile,
  calculateDisplacementMap,
  calculateSpecular,
  imageDataToUrl,
} from "../lib/oxglass";
import type { ShapeType } from "../lib/oxglass/displacementMap";

export interface UseOxGlassOptions {
  surface?: "convex" | "circle" | "concave" | "lip";
  shape?: ShapeType;
  bezel?: number;
  radius?: number;
  glassThickness?: number;
  refraction?: number;
  blur?: number;
  scaleRatio?: number;
  specularOpacity?: number;
  specularSaturation?: number;
  squircleExponent?: number;
  cornerRadius?: number;
  bgOpacity?: number;
}

export function useOxGlass(options: UseOxGlassOptions = {}) {
  const {
    surface = "convex",
    shape = "pill",
    bezel = 40,
    radius,
    glassThickness = 120,
    refraction = 1.5,
    blur = 0.2,
    scaleRatio = 1,
    specularOpacity = 0.4,
    specularSaturation = 4,
    squircleExponent = 2,
    cornerRadius = 1.0,
    bgOpacity = 0.12,
  } = options;

  const rawId = useId();
  const filterId = `oxg-${rawId.replace(/:/g, "")}`;
  const ref = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      const rect = entries[0]?.contentRect;
      if (!rect) return;
      const w = Math.round(rect.width);
      const h = Math.round(rect.height);
      setSize((prev) => (prev.w === w && prev.h === h ? prev : { w, h }));
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const effectiveRadius = radius ?? Math.min(size.w, size.h) / 2;

  const filterData = useMemo(() => {
    if (size.w === 0 || size.h === 0) return null;

    const surfaceFn = SURFACES[surface] ?? SURFACES.convex;
    const profile = calculateDisplacementProfile(glassThickness, bezel, surfaceFn, refraction);
    const maxDisp = Math.max(...profile.map((x) => Math.abs(x))) || 1;

    const { imageData: dispData } = calculateDisplacementMap(
      size.w, size.h, bezel, profile, shape, cornerRadius, squircleExponent,
    );
    const specData = calculateSpecular(
      size.w, size.h, effectiveRadius, bezel, Math.PI / 3,
    );

    return {
      dispUrl: imageDataToUrl(dispData),
      specUrl: imageDataToUrl(specData),
      scale: maxDisp * scaleRatio,
      w: size.w,
      h: size.h,
    };
  }, [size.w, size.h, bezel, refraction, surface, shape, cornerRadius,
      squircleExponent, glassThickness, scaleRatio, effectiveRadius]);

  const glassStyle: React.CSSProperties = filterData
    ? {
        backdropFilter: `url(#${filterId})`,
        WebkitBackdropFilter: `url(#${filterId})`,
        backgroundColor: `rgba(255, 255, 255, ${bgOpacity})`,
      }
    : {
        backdropFilter: `blur(${Math.max(blur * 20, 40)}px) saturate(180%)`,
        WebkitBackdropFilter: `blur(${Math.max(blur * 20, 40)}px) saturate(180%)`,
        backgroundColor: `rgba(255, 255, 255, ${bgOpacity})`,
      };

  return {
    ref,
    filterId,
    filterData,
    glassStyle,
    blur,
    specularOpacity,
    specularSaturation,
  };
}
