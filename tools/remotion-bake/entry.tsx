// Titik masuk bundel: render satu frame komposisi kepada markup, tanpa pelayar.
// Konteks Remotion yang biasanya disediakan Studio/renderer dibekalkan di sini.
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { CanUseRemotionHooks } from "remotion-cjs/CanUseRemotionHooks";
import { VideoContext } from "remotion-cjs/use-video";
import { TimelineContext, AbsoluteTimeContext, PlaybackRateContext } from "remotion-cjs/TimelineContext";
import { COMP } from "bake-target";

const { id, component: C, width, height, fps, durationInFrames } = COMP;
const video = { id, width, height, fps, durationInFrames, defaultProps: {}, props: {}, defaultCodec: null, defaultOutName: null,
  defaultVideoImageFormat: null, defaultPixelFormat: null, defaultProResProfile: null, defaultSampleRate: null, component: C };

export const meta = { id, width, height, fps, durationInFrames };
/** Rekod audio frame yang baru dirender (lihat shim-media.tsx). */
export function takeAudio(): unknown[] {
  const g = globalThis as { __novaAudio?: unknown[] };
  const a = g.__novaAudio ?? [];
  g.__novaAudio = [];
  return a;
}
export function renderFrame(frame: number): string {
  (globalThis as { __novaAudio?: unknown[] }).__novaAudio = [];
  const tl = { frame: { [id]: frame }, playing: false, rootId: "", playbackRate: 1, imperativePlaying: { current: false },
    setPlaybackRate: () => undefined, audioAndVideoTags: { current: [] }, isInsideFreeze: false };
  return renderToStaticMarkup(
    <CanUseRemotionHooks.Provider value={true}>
      <VideoContext.Provider value={video as never}>
        <AbsoluteTimeContext.Provider value={tl as never}>
          <PlaybackRateContext.Provider value={{ playbackRate: 1 } as never}>
            <TimelineContext.Provider value={tl as never}>
              <C />
            </TimelineContext.Provider>
          </PlaybackRateContext.Provider>
        </AbsoluteTimeContext.Provider>
      </VideoContext.Provider>
    </CanUseRemotionHooks.Provider>,
  );
}
