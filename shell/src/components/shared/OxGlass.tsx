import { useId, useRef, useState, useEffect, useMemo } from "react";
import {
  SURFACES,
  calculateDisplacementProfile,
  calculateDisplacementMap,
  calculateSpecular,
  imageDataToUrl,
} from "../../lib/oxglass";
import type { ShapeType } from "../../lib/oxglass/displacementMap";

export interface OxGlassProps {
  children: React.ReactNode;
  className?: string;
  style?: React.CSSProperties;
  /** Glass profile shape */
  surface?: "convex" | "circle" | "concave" | "lip";
  /** Shape geometry for displacement map */
  shape?: ShapeType;
  /** Bezel width in px (refraction edge thickness) */
  bezel?: number;
  /** Corner radius in px */
  radius?: number;
  /** Glass thickness for refraction depth */
  glassThickness?: number;
  /** Refractive index (1.0 = no refraction, 1.5 = glass) */
  refraction?: number;
  /** Blur stdDeviation for the gaussian blur pass */
  blur?: number;
  /** Scale ratio for displacement */
  scaleRatio?: number;
  /** Specular highlight opacity 0-1 */
  specularOpacity?: number;
  /** Saturation boost for the specular layer */
  specularSaturation?: number;
  /** Squircle exponent for corner shape */
  squircleExponent?: number;
  /** Corner radius ratio 0-1 for rectangle/squircle shapes */
  cornerRadius?: number;
  /** Background opacity */
  bgOpacity?: number;
  onPointerDown?: React.PointerEventHandler;
  onPointerMove?: React.PointerEventHandler;
  onPointerUp?: React.PointerEventHandler;
  onClick?: React.MouseEventHandler;
}

export default function OxGlass({
  children,
  className = "",
  style,
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
  onPointerDown,
  onPointerMove,
  onPointerUp,
  onClick,
}: OxGlassProps) {
  const rawId = useId();
  const filterId = rawId.replace(/:/g, "");
  const containerRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });

  // Track container size
  useEffect(() => {
    const el = containerRef.current;
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

  // Compute the actual radius — default to half the shorter dimension
  const effectiveRadius = radius ?? Math.min(size.w, size.h) / 2;

  // Generate filter assets
  const filterData = useMemo(() => {
    if (size.w === 0 || size.h === 0) return null;

    const surfaceFn = SURFACES[surface] ?? SURFACES.convex;

    // Phase 1: 1D displacement profile
    const profile = calculateDisplacementProfile(
      glassThickness, bezel, surfaceFn, refraction,
    );

    const maxDisp = Math.max(...profile.map((x) => Math.abs(x))) || 1;

    // Phase 2: 2D displacement map
    const { imageData: dispData } = calculateDisplacementMap(
      size.w, size.h, bezel, profile, shape, cornerRadius, squircleExponent,
    );

    // Specular highlights
    const specData = calculateSpecular(
      size.w, size.h, effectiveRadius, bezel, Math.PI / 3,
    );

    return {
      dispUrl: imageDataToUrl(dispData),
      specUrl: imageDataToUrl(specData),
      scale: maxDisp * scaleRatio,
      w: dispData.width,
      h: dispData.height,
    };
  }, [size.w, size.h, bezel, refraction, surface, shape, cornerRadius,
      squircleExponent, glassThickness, scaleRatio, effectiveRadius]);

  const hasFilter = filterData !== null;

  const backdropStyle: React.CSSProperties = hasFilter
    ? {
        backdropFilter: `url(#oxg-${filterId})`,
        WebkitBackdropFilter: `url(#oxg-${filterId})`,
      }
    : {
        backdropFilter: `blur(${Math.max(blur * 20, 40)}px) saturate(180%)`,
        WebkitBackdropFilter: `blur(${Math.max(blur * 20, 40)}px) saturate(180%)`,
      };

  return (
    <>
      {hasFilter && (
        <svg
          width="0"
          height="0"
          style={{ position: "absolute", pointerEvents: "none" }}
          aria-hidden
          colorInterpolationFilters="sRGB"
        >
          <defs>
            <filter id={`oxg-${filterId}`} x="0" y="0" width="100%" height="100%">
              {/* Blur */}
              <feGaussianBlur
                in="SourceGraphic"
                stdDeviation={blur}
                result="blurred_source"
              />

              {/* Displacement map */}
              <feImage
                href={filterData.dispUrl}
                x="0"
                y="0"
                width={filterData.w}
                height={filterData.h}
                result="displacement_map"
              />
              <feDisplacementMap
                in="blurred_source"
                in2="displacement_map"
                scale={filterData.scale}
                xChannelSelector="R"
                yChannelSelector="G"
                result="displaced"
              />

              {/* Saturation */}
              <feColorMatrix
                in="displaced"
                type="saturate"
                values={String(specularSaturation)}
                result="displaced_saturated"
              />

              {/* Specular layer */}
              <feImage
                href={filterData.specUrl}
                x="0"
                y="0"
                width={filterData.w}
                height={filterData.h}
                result="specular_layer"
              />
              <feComposite
                in="displaced_saturated"
                in2="specular_layer"
                operator="in"
                result="specular_saturated"
              />
              <feComponentTransfer in="specular_layer" result="specular_faded">
                <feFuncA type="linear" slope={specularOpacity} />
              </feComponentTransfer>

              {/* Blend layers */}
              <feBlend
                in="specular_saturated"
                in2="displaced"
                mode="normal"
                result="withSaturation"
              />
              <feBlend
                in="specular_faded"
                in2="withSaturation"
                mode="normal"
              />
            </filter>
          </defs>
        </svg>
      )}
      <div
        ref={containerRef}
        className={className}
        style={{
          ...backdropStyle,
          backgroundColor: `rgba(255, 255, 255, ${bgOpacity})`,
          ...style,
        }}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onClick={onClick}
      >
        {children}
      </div>
    </>
  );
}
