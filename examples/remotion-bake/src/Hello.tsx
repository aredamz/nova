import React from "react";
import { AbsoluteFill, Sequence, interpolate, spring, useCurrentFrame, useVideoConfig } from "remotion";

export const DURATION = 120;

// Geometry that is recomputed by code on every frame: the case the frame bake exists for.
const Wave: React.FC<{ phase: number; y: number; color: string }> = ({ phase, y, color }) => {
  let d = `M 0 ${y}`;
  for (let x = 0; x <= 1280; x += 20) {
    d += ` L ${x} ${(y + Math.sin(x / 90 + phase) * 28 + Math.sin(x / 37 - phase * 1.7) * 9).toFixed(2)}`;
  }
  return <path d={`${d} L 1280 720 L 0 720 Z`} fill={color} />;
};

const Title: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const pop = spring({ frame, fps, config: { damping: 14 } });
  return (
    <AbsoluteFill style={{ justifyContent: "center", alignItems: "center" }}>
      <div
        style={{
          fontFamily: "Noto Sans",
          fontWeight: 700,
          fontSize: 96,
          color: "#ffffff",
          opacity: interpolate(frame, [0, 12], [0, 1], { extrapolateRight: "clamp" }),
          transform: `scale(${0.7 + 0.3 * pop})`,
        }}
      >
        Rendered by NOVA
      </div>
      <div style={{ fontFamily: "Noto Sans", fontWeight: 400, fontSize: 34, color: "#9fb4ff", marginTop: 18 }}>
        React runs in Node; no browser is involved
      </div>
    </AbsoluteFill>
  );
};

export const Hello: React.FC = () => {
  const frame = useCurrentFrame();
  const t = frame / 30;
  return (
    <AbsoluteFill style={{ backgroundColor: "#0b1020" }}>
      <svg width={1280} height={720} viewBox="0 0 1280 720" style={{ position: "absolute", top: 0, left: 0 }}>
        <defs>
          <radialGradient id="glow" cx="50%" cy="40%" r="60%">
            <stop offset="0%" stopColor="#3350c8" stopOpacity={0.9} />
            <stop offset="100%" stopColor="#0b1020" stopOpacity={0} />
          </radialGradient>
        </defs>
        <rect width={1280} height={720} fill="url(#glow)" />
        {Array.from({ length: 24 }, (_, i) => {
          const a = (i / 24) * Math.PI * 2 + t * 0.6;
          const r = 250 + Math.sin(t * 2 + i) * 30;
          return <circle key={i} cx={640 + Math.cos(a) * r} cy={300 + Math.sin(a) * r * 0.45} r={5 + (i % 3) * 2} fill="#ffd166" opacity={0.85} />;
        })}
        <Wave phase={t * 1.4} y={560} color="#16245c" />
        <Wave phase={t * 2.1 + 2} y={610} color="#1f3a93" />
      </svg>
      <Sequence from={15}>
        <Title />
      </Sequence>
    </AbsoluteFill>
  );
};
