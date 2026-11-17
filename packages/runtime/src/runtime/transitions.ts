import type { MirrorElement, MirrorRect } from "./types";
import { nodeOps } from "./node-ops";
import { writeClass } from "./patch-prop";
import {
  BaseTransition,
  BaseTransitionPropsValidators,
  Fragment,
  createVNode,
  defineComponent,
  getCurrentInstance,
  getTransitionRawChildren,
  onUpdated,
  h,
  resolveTransitionHooks,
  setTransitionHooks,
  toRaw,
  useTransitionState,
} from "@vue/runtime-core";
import type { VNode } from "@vue/runtime-core";

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

type CssKind = "transition" | "animation";
interface TransitionInfo {
  type: CssKind | null;
  timeout: number;
  propCount: number;
  hasTransform: boolean;
}

function computedStyle(el: MirrorElement): Record<string, string> {
  // `getComputedStyle` is installed by this runtime, so it is off the engine's declared surface; hence the cast.
  return (
    globalThis as unknown as { getComputedStyle: (el: MirrorElement) => Record<string, string> }
  ).getComputedStyle(el);
}

/** runtime-dom's `getTransitionInfo`: which of the CSS transitions and animations an element runs ends last, or `expected`'s. */
function transitionInfo(el: MirrorElement, expected?: string): TransitionInfo {
  const styles = computedStyle(el);
  const split = (k: string) => String(styles[k] || "").split(", ");
  const transitionDurations = split("transitionDuration");
  const transitionTimeout = listTimeout(split("transitionDelay"), transitionDurations);
  const animationDurations = split("animationDuration");
  const animationTimeout =
    String(styles.animationName || "none") === "none" ? 0 : listTimeout(split("animationDelay"), animationDurations);
  let type: CssKind | null = null;
  let timeout = 0;
  let propCount = 0;
  if (expected === "transition") {
    if (transitionTimeout > 0)
      [type, timeout, propCount] = ["transition", transitionTimeout, transitionDurations.length];
  } else if (expected === "animation") {
    if (animationTimeout > 0) [type, timeout, propCount] = ["animation", animationTimeout, animationDurations.length];
  } else {
    timeout = Math.max(transitionTimeout, animationTimeout);
    type = timeout > 0 ? (transitionTimeout > animationTimeout ? "transition" : "animation") : null;
    propCount =
      type === "transition" ? transitionDurations.length : type === "animation" ? animationDurations.length : 0;
  }
  const hasTransform = type === "transition" && /\b(transform|all)(,|$)/.test(String(styles.transitionProperty || ""));
  return { type, timeout, propCount, hasTransform };
}

let endId = 0;
/// Ends a phase when the element's own `transitionend`s or `animationend`s have all arrived, with the computed timeout
/// as a safety net (the timer alone reports the declared duration even when the engine cut the run short).
function whenTransitionEnds(
  el: MirrorElement,
  expectedType: string | undefined,
  explicitTimeout: number | null | undefined,
  resolve: () => void,
): void {
  const id = (el._endId = ++endId);
  let done = false;
  const resolveIfNotStale = () => {
    if (done || id !== el._endId) return;
    done = true;
    resolve();
  };
  if (explicitTimeout != null) {
    setTimeout(resolveIfNotStale, explicitTimeout);
    return;
  }
  const { type, timeout, propCount } = transitionInfo(el, expectedType);
  if (!type) {
    resolve();
    return;
  }
  const endEvent = `${type}end`;
  let ended = 0;
  const finish = () => {
    el.removeEventListener?.(endEvent, onEnd);
    resolveIfNotStale();
  };
  const onEnd = (e: Event) => {
    // Only this element's own runs end the phase; a descendant's bubbles through.
    if (e.target === el && ++ended >= propCount) finish();
  };
  el.addEventListener?.(endEvent, onEnd);
  setTimeout(() => {
    if (ended < propCount) finish();
  }, timeout + 1);
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
    type,
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
      if (!hasExplicitCallback(hook)) whenTransitionEnds(el, type, enterDuration, resolve);
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
        if (!hasExplicitCallback(onLeave)) whenTransitionEnds(el, type, leaveDuration, resolve);
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
    const instance = getCurrentInstance()!;
    const state = useTransitionState();
    let prevChildren: VNode[] = [];
    let children: VNode[] | undefined;

    // FLIP, as runtime-dom does it: each item's box before the patch (read in render), after it (a geometry read
    // lays the reordered list out), the difference put back as a transform, then the move class animates it away.
    onUpdated(() => {
      const moved = prevChildren;
      prevChildren = [];
      if (!moved.length) return;
      const rawProps = toRaw(props) as TransitionRawProps & { moveClass?: string };
      const moveClass = rawProps.moveClass || `${rawProps.name || "v"}-move`;
      const first = moved[0]!.el as MirrorElement;
      if (!hasCssTransform(first, instance.vnode.el as MirrorElement | null, moveClass)) return;
      for (const c of moved) ((c.el as MirrorElement)[moveCbKey] as (() => void) | undefined)?.();
      for (const c of moved) newPositions.set(c, (c.el as MirrorElement).getBoundingClientRect());
      const shifted = moved.filter(applyTranslation);
      if (!shifted.length) return;
      // The forced style flush a browser's `offsetHeight` read makes: the items start from their old place.
      void computedStyle(shifted[0]!.el as MirrorElement).transform;
      for (const c of shifted) {
        const el = c.el as MirrorElement;
        addTransitionClass(el, moveClass);
        const style = (el as unknown as { style: Record<string, string> }).style;
        style.transform = "";
        style.transitionDuration = "";
        const done = (e?: Event) => {
          if (e && e.target !== el) return;
          if (!e || String((e as TransitionEvent).propertyName).endsWith("transform")) {
            el.removeEventListener("transitionend", done);
            el[moveCbKey] = undefined;
            removeTransitionClass(el, moveClass);
          }
        };
        el[moveCbKey] = done;
        el.addEventListener("transitionend", done);
      }
    });

    return () => {
      const rawProps = toRaw(props) as TransitionRawProps;
      const cssTransitionProps = resolveTransitionProps(rawProps);
      const tag = rawProps.tag || Fragment;
      prevChildren = [];
      for (const child of children ?? []) {
        if (child.el && (child.el as MirrorElement).kind === 1) {
          prevChildren.push(child);
          setTransitionHooks(child, resolveTransitionHooks(child, cssTransitionProps, state, instance));
          positions.set(child, (child.el as MirrorElement).getBoundingClientRect());
        }
      }
      children = slots.default ? getTransitionRawChildren(slots.default()) : [];
      for (const child of children) {
        if (child.key != null) {
          setTransitionHooks(child, resolveTransitionHooks(child, cssTransitionProps, state, instance));
        }
      }
      return createVNode(tag as never, null, children as never);
    };
  },
});

const moveCbKey = Symbol("skMoveCb");
const positions = new WeakMap<VNode, MirrorRect>();
const newPositions = new WeakMap<VNode, MirrorRect>();

function applyTranslation(c: VNode): boolean {
  const before = positions.get(c);
  const after = newPositions.get(c);
  if (!before || !after) return false;
  const dx = before.left - after.left;
  const dy = before.top - after.top;
  if (!dx && !dy) return false;
  const style = (c.el as { style: Record<string, string> }).style;
  style.transform = `translate(${dx}px,${dy}px)`;
  style.transitionDuration = "0s";
  return true;
}

/** Whether the move class transitions `transform`, read off a hidden clone so the item's own style does not change. */
function hasCssTransform(el: MirrorElement, root: MirrorElement | null, moveClass: string): boolean {
  const clone = el.cloneNode() as MirrorElement;
  const vtc = el._vtc;
  const classes = String(el._class || "")
    .split(/\s+/)
    .filter((c) => c && !vtc?.has(c));
  clone._vtc = undefined;
  (clone as unknown as { className: string }).className = [...classes, ...moveClass.split(/\s+/)]
    .filter(Boolean)
    .join(" ");
  (clone as unknown as { style: Record<string, string> }).style.display = "none";
  const container = (root && root.kind === 1 ? root : el.parentNode) as MirrorElement | null;
  if (!container) return false;
  nodeOps.insert(clone, container, null);
  const { hasTransform } = transitionInfo(clone);
  nodeOps.remove(clone);
  return hasTransform;
}
