import { createPortal } from "react-dom";

interface OxGlassFilterProps {
  filterId: string;
  filterData: {
    dispUrl: string;
    specUrl: string;
    scale: number;
    w: number;
    h: number;
  };
  blur: number;
  specularOpacity: number;
  specularSaturation: number;
}

/**
 * Renders the SVG filter definition at the document body level via portal.
 * This prevents the filter from being nested inside the glass element,
 * which would cause visual layering artifacts.
 */
export default function OxGlassFilter({
  filterId,
  filterData,
  blur,
  specularOpacity,
  specularSaturation,
}: OxGlassFilterProps) {
  return createPortal(
    <svg
      width="0"
      height="0"
      style={{ position: "absolute", pointerEvents: "none" }}
      aria-hidden
      colorInterpolationFilters="sRGB"
    >
      <defs>
        <filter id={filterId} x="0" y="0" width="100%" height="100%">
          <feGaussianBlur in="SourceGraphic" stdDeviation={blur} result="blurred_source" />
          <feImage href={filterData.dispUrl} x="0" y="0" width={filterData.w} height={filterData.h} result="displacement_map" />
          <feDisplacementMap in="blurred_source" in2="displacement_map" scale={filterData.scale} xChannelSelector="R" yChannelSelector="G" result="displaced" />
          <feColorMatrix in="displaced" type="saturate" values={String(specularSaturation)} result="displaced_saturated" />
          <feImage href={filterData.specUrl} x="0" y="0" width={filterData.w} height={filterData.h} result="specular_layer" />
          <feComposite in="displaced_saturated" in2="specular_layer" operator="in" result="specular_saturated" />
          <feComponentTransfer in="specular_layer" result="specular_faded">
            <feFuncA type="linear" slope={specularOpacity} />
          </feComponentTransfer>
          <feBlend in="specular_saturated" in2="displaced" mode="normal" result="withSaturation" />
          <feBlend in="specular_faded" in2="withSaturation" mode="normal" />
        </filter>
      </defs>
    </svg>,
    document.body,
  );
}
