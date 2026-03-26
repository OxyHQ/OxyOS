import { useOxGlass } from "../../hooks/useOxGlass";
import type { UseOxGlassOptions } from "../../hooks/useOxGlass";
import OxGlassFilter from "./OxGlassFilter";

export interface OxGlassProps extends UseOxGlassOptions {
  children: React.ReactNode;
  className?: string;
  style?: React.CSSProperties;
  onPointerDown?: React.PointerEventHandler;
  onPointerMove?: React.PointerEventHandler;
  onPointerUp?: React.PointerEventHandler;
  onClick?: React.MouseEventHandler;
}

/**
 * OxGlass wrapper component — makes its root div a liquid glass surface.
 * The SVG filter is portaled to document.body so it doesn't nest inside.
 *
 * For elements that can't use a wrapper div (e.g. motion.div), use the
 * useOxGlass() hook directly and render <OxGlassFilter> alongside.
 */
export default function OxGlass({
  children,
  className = "",
  style,
  onPointerDown,
  onPointerMove,
  onPointerUp,
  onClick,
  ...glassOptions
}: OxGlassProps) {
  const { ref, filterId, filterData, glassStyle, blur, specularOpacity, specularSaturation } =
    useOxGlass(glassOptions);

  return (
    <>
      {filterData && (
        <OxGlassFilter
          filterId={filterId}
          filterData={filterData}
          blur={blur}
          specularOpacity={specularOpacity}
          specularSaturation={specularSaturation}
        />
      )}
      <div
        ref={ref}
        className={className}
        style={{ ...glassStyle, ...style }}
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
