import type { FunctionalComponent } from "@vue/runtime-core";

// The element types Vue templates are checked against: vue-tsc reads `JSX.IntrinsicElements` from
// `<vueCompilerOptions.lib>/jsx-runtime`. Attributes not listed are accepted unchecked.

type Booleanish = boolean | "true" | "false" | "";
type Numberish = number | `${number}`;

type Handlers<M> = { [K in keyof M as `on${Capitalize<K & string>}`]?: ((event: M[K]) => void) | undefined };

type ElementAttributes<M = {}> = Handlers<Omit<GlobalEventHandlersEventMap, keyof M> & M> & {
  [attribute: string]: unknown;
};

type SizeHints = { width?: Numberish | undefined; height?: Numberish | undefined };

interface SkMediaEventMap {
  loadstart: SkMediaEvent;
  loadedmetadata: SkMediaEvent;
  loadeddata: SkMediaEvent;
  canplay: SkMediaEvent;
  canplaythrough: SkMediaEvent;
  play: SkMediaEvent;
  playing: SkMediaEvent;
  pause: SkMediaEvent;
  seeking: SkMediaEvent;
  seeked: SkMediaEvent;
  timeupdate: SkMediaEvent;
  durationchange: SkMediaEvent;
  ratechange: SkMediaEvent;
  volumechange: SkMediaEvent;
  waiting: SkMediaEvent;
  ended: SkMediaEvent;
  error: SkMediaEvent;
  emptied: SkMediaEvent;
}

type MediaAttributes = ElementAttributes<SkMediaEventMap> &
  SizeHints & {
    src?: string | undefined;
    autoplay?: Booleanish | undefined;
    loop?: Booleanish | undefined;
    muted?: Booleanish | undefined;
    volume?: Numberish | undefined;
    playbackrate?: Numberish | undefined;
    currenttime?: Numberish | undefined;
  };

interface SkElements {
  audio: MediaAttributes;
  video: MediaAttributes & { poster?: string | undefined; playsinline?: Booleanish | undefined };
  /** Shows the frames native code presents to the surface `name` (Rust: `skathi::surface(name)`). */
  "sk-surface": ElementAttributes & SizeHints & { name?: string | undefined };
  /** Plays a Lottie animation (`.json` or `.lottie`). */
  lottie: ElementAttributes<{
    enterFrame: SkLottieFrameEvent;
    loopComplete: SkLottieLoopEvent;
    complete: SkLottieCompleteEvent;
  }> &
    SizeHints & {
      src?: string | undefined;
      autoplay?: Booleanish | undefined;
      /** Present: loop forever. A number `n`: play `n` more times. */
      loop?: Booleanish | Numberish | undefined;
      speed?: Numberish | undefined;
      direction?: 1 | -1 | "1" | "-1" | "forward" | "reverse" | undefined;
      mode?: "normal" | "bounce" | undefined;
      /** Two source frame numbers: `"30,59"`. */
      segment?: string | undefined;
      objectfit?: "fill" | "contain" | "cover" | "scale-down" | "none" | undefined;
    };
  /** An animated sticker from Lottie (`.json`, `.tgs`) or WebM with alpha; copies of one source share a decode. */
  "sk-sticker": ElementAttributes<{ error: Event; ended: Event }> &
    SizeHints & {
      src?: string | undefined;
      play?: "auto" | "on" | "off" | undefined;
      once?: Booleanish | undefined;
      tint?: Booleanish | undefined;
      "max-fps"?: Numberish | undefined;
      maxFps?: Numberish | undefined;
      kind?: "auto" | "lottie" | "video" | "image" | undefined;
    };
}

type Tag = keyof HTMLElementTagNameMap | keyof SVGElementTagNameMap;

export declare namespace JSX {
  type IntrinsicElements = { [K in Exclude<Tag, keyof SkElements>]: ElementAttributes } & SkElements & {
      [tag: string]: ElementAttributes;
    };
}

// Vue's template compiler takes a tag it does not know as HTML or SVG for a component, so these are typed there too.
declare module "@vue/runtime-core" {
  interface GlobalComponents {
    lottie: FunctionalComponent<SkElements["lottie"]>;
    "sk-surface": FunctionalComponent<SkElements["sk-surface"]>;
    "sk-sticker": FunctionalComponent<SkElements["sk-sticker"]>;
  }
}
