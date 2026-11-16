import type { MirrorElement } from "./types";
import { writeClass } from "./patch-prop";
import {
  BaseTransition,
  BaseTransitionPropsValidators,
  Fragment,
  createVNode,
  defineComponent,
  getCurrentInstance,
  getTransitionRawChildren,
  h,
  resolveTransitionHooks,
  setTransitionHooks,
  toRaw,
  useTransitionState,
} from "@vue/runtime-core";
import type { VNode } from "@vue/runtime-core";
import { warnCompat } from "../compat";

// `<Transition>`/`<TransitionGroup>` live in runtime-dom, but libraries import them from "vue" at module scope, so
// their absence breaks builds. Ported onto our transition engine.

const TRANSITION_PROPS = {
  name: String,
  type: String,
  css: { type: Boolean, default: true },
  duration: [String, Number, Object],
  enterFromClass: String,
  enterActiveClass: String,
  enterToClass: String,
  appearFromClass: String,
  appearActiveClass: String,
  appearToClass: String,
  leaveFromClass: String,
  leaveActiveClass: String,
  leaveToClass: String,
};

// runtime-dom records the phase class in `_vtc` and `classList`; here `writeClass` joins two
// sources, so record it once or it is emitted twice.
function addTransitionClass(el: MirrorElement, cls: string): void {
  (el._vtc || (el._vtc = new Set())).add(cls);
  writeClass(el);
}
function removeTransitionClass(el: MirrorElement, cls: string): void {
  if (el._vtc) {
    el._vtc.delete(cls);
    if (!el._vtc.size) el._vtc = undefined;
  }
  const base = (el._class || "").split(/\s+/);
  if (base.includes(cls)) el._class = base.filter((c) => c && c !== cls).join(" ");
  writeClass(el);
}

/// Two frames: one to commit the *-from class, the next to swap in *-to so the engine sees a real change.
function nextFrame(cb: () => void): void {
  const raf = globalThis.requestAnimationFrame;
  if (typeof raf === "function") raf(() => raf(cb));
  else setTimeout(() => setTimeout(cb, 0), 0);
}

const toMs = (s: string): number => (s === "auto" || !s ? 0 : Number(String(s).slice(0, -1).replace(",", ".")) * 1000);
function listTimeout(delays: string[], durations: string[]): number {
  while (delays.length < durations.length) delays = delays.concat(delays);
  return Math.max(0, ...durations.map((d: string, i: number) => toMs(d) + toMs(delays[i]!)));
}

function transitionTimeout(el: MirrorElement): number {
  // `getComputedStyle` is installed by this file, so it is off the engine's declared surface; hence the cast.
  const styles = (
    globalThis as unknown as { getComputedStyle: (el: MirrorElement) => Record<string, string> }
  ).getComputedStyle(el);
  const split = (k: string) => String(styles[k] || "").split(", ");
  return listTimeout(split("transitionDelay"), split("transitionDuration"));
}

let endId = 0;
/// Ends a phase on the real `transitionend`, with the computed timeout as a safety net (the timer alone reports the
/// declared duration even when the engine cut the transition short).
function whenTransitionEnds(el: MirrorElement, explicitTimeout: number | null | undefined, resolve: () => void): void {
  const id = (el._endId = ++endId);
  let done = false;
  const resolveIfNotStale = () => {
    if (done || id !== el._endId) return;
    done = true;
    el.removeEventListener?.("transitionend", onEnd);
    resolve();
  };
  const onEnd = (e: Event) => {
    // Only this element's own transitions end the phase; a descendant's bubbles through.
    if (e.target === el) resolveIfNotStale();
  };
  if (explicitTimeout != null) {
    setTimeout(resolveIfNotStale, explicitTimeout);
    return;
  }
  const timeout = transitionTimeout(el);
  if (!timeout) {
    resolve();
    return;
  }
  el.addEventListener?.("transitionend", onEnd);
  setTimeout(resolveIfNotStale, timeout + 1);
}

const callHook = (hook: unknown, args: unknown[] = []) => {
  if (Array.isArray(hook)) for (const h2 of hook) (h2 as (...a: unknown[]) => void)(...args);
  else if (hook) (hook as (...a: unknown[]) => void)(...args);
};
const hasExplicitCallback = (hook: unknown): boolean =>
  hook
    ? Array.isArray(hook)
      ? hook.some((h2) => (h2 as { length: number }).length > 1)
      : (hook as { length: number }).length > 1
    : false;

type DurationPair = { enter: unknown; leave: unknown };

function normalizeDuration(duration: unknown): [number, number] | null {
  if (duration == null) return null;
  if (typeof duration === "object")
    return [Number((duration as DurationPair).enter), Number((duration as DurationPair).leave)];
  const n = Number(duration);
  return [n, n];
}

interface TransitionRawProps {
  name?: string;
  type?: string;
  css?: boolean;
  duration?: number | string | { enter: number; leave: number };
  enterFromClass?: string;
  enterActiveClass?: string;
  enterToClass?: string;
  appearFromClass?: string;
  appearActiveClass?: string;
  appearToClass?: string;
  leaveFromClass?: string;
  leaveActiveClass?: string;
  leaveToClass?: string;
  [key: string]: unknown;
}

export function resolveTransitionProps(rawProps: TransitionRawProps) {
  const baseProps: Record<string, unknown> = {};
  for (const key in rawProps) if (!(key in TRANSITION_PROPS)) baseProps[key] = rawProps[key];
  if (rawProps.css === false) return baseProps;
  const {
    name = "v",
    duration,
    enterFromClass = `${name}-enter-from`,
    enterActiveClass = `${name}-enter-active`,
    enterToClass = `${name}-enter-to`,
    appearFromClass = enterFromClass,
    appearActiveClass = enterActiveClass,
    appearToClass = enterToClass,
    leaveFromClass = `${name}-leave-from`,
    leaveActiveClass = `${name}-leave-active`,
    leaveToClass = `${name}-leave-to`,
  } = rawProps;
  const durations = normalizeDuration(duration);
  const enterDuration = durations && durations[0];
  const leaveDuration = durations && durations[1];
  const {
    onBeforeEnter,
    onEnter,
    onEnterCancelled,
    onLeave,
    onLeaveCancelled,
    onBeforeAppear = onBeforeEnter,
    onAppear = onEnter,
    onAppearCancelled = onEnterCancelled,
  } = baseProps;

  const finishEnter = (el: MirrorElement, isAppear: boolean, done: (() => void) | undefined) => {
    removeTransitionClass(el, isAppear ? appearToClass : enterToClass);
    removeTransitionClass(el, isAppear ? appearActiveClass : enterActiveClass);
    done && done();
  };
  const finishLeave = (el: MirrorElement, done?: () => void) => {
    el._isLeaving = false;
    removeTransitionClass(el, leaveFromClass);
    removeTransitionClass(el, leaveToClass);
    removeTransitionClass(el, leaveActiveClass);
    done && done();
  };
  const makeEnterHook = (isAppear: boolean) => (el: MirrorElement, done: () => void) => {
    const hook = isAppear ? onAppear : onEnter;
    const resolve = () => finishEnter(el, isAppear, done);
    callHook(hook, [el, resolve]);
    nextFrame(() => {
      removeTransitionClass(el, isAppear ? appearFromClass : enterFromClass);
      addTransitionClass(el, isAppear ? appearToClass : enterToClass);
      if (!hasExplicitCallback(hook)) whenTransitionEnds(el, enterDuration, resolve);
    });
  };

  return Object.assign(baseProps, {
    onBeforeEnter(el: MirrorElement) {
      callHook(onBeforeEnter, [el]);
      addTransitionClass(el, enterFromClass);
      addTransitionClass(el, enterActiveClass);
    },
    onBeforeAppear(el: MirrorElement) {
      callHook(onBeforeAppear, [el]);
      addTransitionClass(el, appearFromClass);
      addTransitionClass(el, appearActiveClass);
    },
    onEnter: makeEnterHook(false),
    onAppear: makeEnterHook(true),
    onLeave(el: MirrorElement, done: () => void) {
      el._isLeaving = true;
      const resolve = () => finishLeave(el, done);
      addTransitionClass(el, leaveFromClass);
      addTransitionClass(el, leaveActiveClass);
      nextFrame(() => {
        if (!el._isLeaving) return;
        removeTransitionClass(el, leaveFromClass);
        addTransitionClass(el, leaveToClass);
        if (!hasExplicitCallback(onLeave)) whenTransitionEnds(el, leaveDuration, resolve);
      });
      callHook(onLeave, [el, resolve]);
    },
    onEnterCancelled(el: MirrorElement) {
      finishEnter(el, false, undefined);
      callHook(onEnterCancelled, [el]);
    },
    onAppearCancelled(el: MirrorElement) {
      finishEnter(el, true, undefined);
      callHook(onAppearCancelled, [el]);
    },
    onLeaveCancelled(el: MirrorElement) {
      finishLeave(el);
      callHook(onLeaveCancelled, [el]);
    },
  });
}

export const Transition = /*@__PURE__*/ defineComponent({
  name: "Transition",
  displayName: "Transition",
  inheritAttrs: false,
  props: /*@__PURE__*/ Object.assign({}, BaseTransitionPropsValidators, TRANSITION_PROPS),
  setup(props, { slots }) {
    return () => h(BaseTransition, resolveTransitionProps(props as TransitionRawProps), slots);
  },
});

export const TransitionGroup = /*@__PURE__*/ defineComponent({
  name: "TransitionGroup",
  props: /*@__PURE__*/ Object.assign({}, BaseTransitionPropsValidators, TRANSITION_PROPS, {
    tag: String,
    moveClass: String,
  }),
  setup(props, { slots }) {
    const instance = getCurrentInstance();
    const state = useTransitionState();
    let children: unknown[] | undefined;
    return () => {
      const rawProps = toRaw(props) as TransitionRawProps;
      const cssTransitionProps = resolveTransitionProps(rawProps);
      const tag = rawProps.tag || Fragment;
      if (rawProps.moveClass || (children && children.length)) {
        warnCompat(
          "TransitionGroup:move",
          "<TransitionGroup> animates enter and leave, but NOT the move of the items that stay — " +
            "`moveClass` is ignored and reordered items jump to their new place",
        );
      }
      children = slots.default ? getTransitionRawChildren(slots.default()) : [];
      for (const child of children as VNode[]) {
        if (child.key != null) {
          setTransitionHooks(child, resolveTransitionHooks(child, cssTransitionProps, state, instance!));
        }
      }
      return createVNode(tag as never, null, children as never);
    };
  },
});
