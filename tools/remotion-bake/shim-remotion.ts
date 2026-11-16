// "remotion" untuk bake tanpa pelayar: semua eksport asal; audio direkod (lihat shim-media.tsx), video tiada.
export * from "remotion-cjs";
import * as R from "remotion-cjs";
import * as M from "./shim-media";
export const Audio = M.Audio as unknown as typeof R.Audio;
export const Html5Audio = M.Html5Audio as unknown as typeof R.Html5Audio;
export const Video = M.Video as unknown as typeof R.Video;
export const OffthreadVideo = M.OffthreadVideo as unknown as typeof R.OffthreadVideo;
export const delayRender = () => 0;
export const continueRender = () => undefined;
export const staticFile = (p: string) => "public/" + p.replace(/^\//, "");
