// Ganti @remotion/skia & @shopify/react-native-skia: keluarkan elemen penanda yang NOVA lukis sendiri.
import React from "react";
const tag = (name: string) => (props: Record<string, unknown>) => {
  const { children, ...rest } = props as { children?: React.ReactNode };
  return React.createElement(name, { "data-props": JSON.stringify(rest, (_k, v) => (v && v.__sksl ? { sksl: v.__sksl } : v)) }, children);
};
export const SkiaCanvas = tag("nova-canvas");
export const Canvas = SkiaCanvas;
export const Fill = tag("nova-fill");
export const Shader = tag("nova-shader");
export const Group = tag("nova-group");
export const Path = tag("nova-path");
export const BlurMask = tag("nova-blurmask");
export const Circle = tag("nova-circle");
export const Rect = tag("nova-rect");
export const Skia = {
  RuntimeEffect: { Make: (src: string) => ({ __sksl: src }) },
  Path: { MakeFromSVGString: (d: string) => ({ svg: d }), Make: () => ({ svg: "" }) },
  Color: (c: unknown) => c,
};
export default {};
