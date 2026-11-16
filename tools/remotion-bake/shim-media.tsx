// <Audio> untuk bake: tidak memainkan apa-apa, hanya MEREKOD apa yang Remotion akan campurkan
// pada frame ini (fail, kedudukan dalam fail, kadar, volum). `mix.mjs` membina trek daripadanya.
// Kiraan masa/volum mengikut @remotion/media `AudioForRendering` (4.0.542).
import { useContext } from "react";
import { Internals, useCurrentFrame } from "remotion-cjs";

export type AudioRec = {
  k: string; src: string; f: number; t: number; r: number; v: number;
  l?: 1; tb?: number; ta?: number; pp?: 1; tone?: number;
};
type Props = {
  src?: string; volume?: number | ((f: number) => number); playbackRate?: number; muted?: boolean;
  loop?: boolean; loopVolumeCurveBehavior?: "repeat" | "extend"; preservePitch?: boolean; toneFrequency?: number;
  trimBefore?: number; trimAfter?: number; startFrom?: number; endAt?: number;
};
const G = globalThis as unknown as { __novaAudio?: AudioRec[]; __novaMediaSeconds?: (src: string) => number };

export const Audio = (p: Props) => {
  const frame = useCurrentFrame();
  const abs = Internals.useTimelinePosition();
  const cfg = Internals.useUnsafeVideoConfig();
  const seq = useContext(Internals.SequenceContext);
  const startsAt = Internals.useMediaStartsAt();
  const out = G.__novaAudio;
  if (!out || !cfg || !p.src || p.muted) return null;
  const fps = cfg.fps;
  const seqRate = seq?.playbackRate ?? 1;
  const rate = (p.playbackRate ?? 1) * seqRate;
  const tb = p.trimBefore ?? p.startFrom, ta = p.trimAfter ?? p.endAt;
  const t = frame / seqRate / fps; // masa tanpa gelung, dalam saat garis masa
  let vf = frame + startsAt;
  if (p.loop && (p.loopVolumeCurveBehavior ?? "repeat") === "repeat" && G.__novaMediaSeconds) {
    // keluk volum berulang setiap gelung: perlukan panjang fail
    const loopFrames = Internals.calculateMediaDuration({ trimAfter: ta, mediaDurationInFrames: G.__novaMediaSeconds(p.src) * fps, playbackRate: 1, trimBefore: tb });
    const inLoop = ((t * rate * fps) % loopFrames) / rate;
    vf = Math.max(0, Math.min(vf, inLoop));
  }
  const v = Internals.evaluateVolume({ volume: p.volume, frame: vf, mediaVolume: 1 });
  if (!(v > 0)) return null;
  const base = `${p.src}|${seq?.cumulatedFrom ?? 0}|${seq?.relativeFrom ?? 0}|${seq?.durationInFrames ?? ""}|${rate}|${tb ?? ""}`;
  let k = base;
  for (let n = 1; out.some((r) => r.k === k); n++) k = `${base}#${n}`; // dua <Audio> serupa dalam satu Sequence
  const rec: AudioRec = { k, src: p.src, f: abs, t, r: rate, v };
  if (p.loop) rec.l = 1;
  if (tb) rec.tb = tb;
  if (ta !== undefined) rec.ta = ta;
  if (p.preservePitch) rec.pp = 1;
  if (p.toneFrequency && p.toneFrequency !== 1) rec.tone = p.toneFrequency;
  out.push(rec);
  return null;
};
export const Html5Audio = Audio;
export const Video = () => null;
export const OffthreadVideo = () => null;
