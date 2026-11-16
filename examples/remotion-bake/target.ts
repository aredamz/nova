// The composition to bake: the same fields a <Composition> takes in a Remotion root.
import { Hello, DURATION } from "./src/Hello";

export const COMP = { id: "Hello", component: Hello, width: 1280, height: 720, fps: 30, durationInFrames: DURATION };
