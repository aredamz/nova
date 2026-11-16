// Rekod audio per frame (daripada shim-media.tsx) -> kiu untuk mix.mjs.
// Satu kiu = satu <Audio> yang berbunyi berterusan, dengan volum setiap frame.
export function audioCues(recs, fps, first) {
  recs.sort((a, b) => (a.k < b.k ? -1 : a.k > b.k ? 1 : a.f - b.f));
  const cues = [];
  let cur = null, last = null;
  for (const r of recs) {
    const joins = last && last.k === r.k && r.f === last.f + 1 && Math.abs(r.t - last.t - 1 / fps) < 1e-6;
    if (!joins) {
      cur = { src: r.src, frame: r.f - first, t: +r.t.toFixed(6), rate: r.r, volumes: [] };
      if (r.l) cur.loop = true;
      if (r.tb) cur.trim_before = r.tb;
      if (r.ta !== undefined) cur.trim_after = r.ta;
      if (r.pp) cur.preserve_pitch = true;
      if (r.tone) cur.tone = r.tone;
      cues.push(cur);
    }
    cur.volumes.push(+r.v.toFixed(5));
    last = r;
  }
  return cues.sort((a, b) => a.frame - b.frame || (a.src < b.src ? -1 : 1));
}
