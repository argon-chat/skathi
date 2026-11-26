import type { MirrorElement } from "./runtime/types";

export type SkMenuBackdrop = "acrylic" | "mica" | "mica-alt" | "transparent" | "none";
export type SkMenuAnchor = Element | MouseEvent;

export interface SkContextMenuHandle<T = unknown> {
  open(anchor: SkMenuAnchor, context?: T): void;
  close(): void;
}

type Location = { anchor: MirrorElement; point?: { x: number; y: number } };
type Level = {
  root: MirrorElement | null;
  nativeId: number;
  at: Location;
  backdrop: SkMenuBackdrop;
  active: MirrorElement | null;
  keyboard: boolean;
  hide(): void;
};
const levels: Level[] = [];
const submenus = new WeakMap<MirrorElement, (keyboard: boolean) => void>();
let hoverTimer: ReturnType<typeof setTimeout> | undefined;

function cancelHover() {
  clearTimeout(hoverTimer);
  hoverTimer = undefined;
}

function setActive(level: Level, item: MirrorElement | null, keyboard = false) {
  item?.toggleAttribute("data-keyboard", keyboard);
  if (level.active === item) return;
  level.active?.removeAttribute("data-active");
  level.active?.removeAttribute("data-keyboard");
  level.active = item;
  if (item) {
    item.setAttribute("data-active", "");
    let id = item.getAttribute("id");
    if (!id) {
      id = "sk-menu-item-" + item.__skId;
      item.setAttribute("id", id);
    }
    level.root?.setAttribute("aria-activedescendant", id);
    if (keyboard)
      requestAnimationFrame(() => {
        if (level.active === item && item.isConnected) item.scrollIntoView({ block: "nearest" });
      });
  } else level.root?.removeAttribute("aria-activedescendant");
}

function items(level: Level): MirrorElement[] {
  const root = level.root;
  if (!root) return [];
  return Array.from(root.querySelectorAll('[role="menuitem"]')).filter(
    (item) =>
      item.closest('[role="menu"]') === root &&
      item.getAttribute("aria-disabled") !== "true" &&
      !item.hasAttribute("hidden"),
  );
}

function closeFrom(i: number) {
  if (i < 0 || i >= levels.length) return;
  cancelHover();
  for (const level of levels.splice(i).reverse()) {
    if (level.nativeId) __sk_context_menu_close(level.nativeId);
    setActive(level, null);
    level.hide();
  }
  if (!levels.length) listen(false);
}

function element(value: unknown): MirrorElement | null {
  return typeof value === "object" && value !== null && "__skId" in value && "closest" in value
    ? (value as MirrorElement)
    : null;
}

function levelFor(el: MirrorElement | null): Level | undefined {
  const root = el?.closest('[role="menu"]');
  return root ? levels.find((level) => level.root === root) : undefined;
}

function highlight(e: Event) {
  cancelHover();
  const target = element(e.target);
  const level = levelFor(target);
  const item = target?.closest('[role="menuitem"]');
  if (!level || !item) return;
  const enabled = item.getAttribute("aria-disabled") !== "true";
  setActive(level, enabled ? item : null);
  const i = levels.indexOf(level);
  if (levels[i + 1]?.at.anchor === item) return;
  hoverTimer = setTimeout(() => {
    closeFrom(i + 1);
    if (enabled) submenus.get(item)?.(false);
  }, 160);
}

function pressOutside(e: Event) {
  if (!levelFor(element(e.target))) closeFrom(0);
}

function keydown(event: Event) {
  const e = event as KeyboardEvent;
  const level = levels[levels.length - 1];
  if (!level || e.altKey || e.metaKey || e.ctrlKey) return;
  if (e.key === "Tab") {
    closeFrom(0);
    return;
  }
  const choices = items(level);
  const index = level.active ? choices.indexOf(level.active) : -1;
  let next: MirrorElement | undefined;
  switch (e.key) {
    case "ArrowDown":
      next = choices[(index + 1) % choices.length];
      break;
    case "ArrowUp":
      next = choices[index <= 0 ? choices.length - 1 : index - 1];
      break;
    case "Home":
      next = choices[0];
      break;
    case "End":
      next = choices[choices.length - 1];
      break;
    case "Escape":
      closeFrom(levels.length - 1);
      break;
    case "ArrowLeft":
      if (levels.length > 1) closeFrom(levels.length - 1);
      break;
    case "ArrowRight":
      if (level.active) submenus.get(level.active)?.(true);
      break;
    case "Enter":
    case " ": {
      if (index >= 0 && level.active) {
        const submenu = submenus.get(level.active);
        if (submenu) submenu(true);
        else level.active.click();
      }
      break;
    }
    default:
      return;
  }
  cancelHover();
  if (next) setActive(level, next, true);
  e.preventDefault();
  e.stopImmediatePropagation();
}

function listen(on: boolean) {
  if (on) {
    Object.assign(globalThis, { __sk_context_menu_dismiss: dismiss });
    document.addEventListener("keydown", keydown, true);
    document.addEventListener("mousedown", pressOutside, true);
  } else {
    document.removeEventListener("keydown", keydown, true);
    document.removeEventListener("mousedown", pressOutside, true);
  }
}

function location(from: SkMenuAnchor): Location | null {
  if (!("button" in from)) {
    const anchor = element(from);
    return anchor ? { anchor } : null;
  }
  const anchor = element(from.button === 2 ? from.currentTarget : from.target) ?? element(from.target);
  if (!anchor) return null;
  from.preventDefault();
  return from.button === 2 ? { anchor, point: { x: from.clientX, y: from.clientY } } : { anchor };
}

export function createContextMenu<T>(
  backdrop: () => SkMenuBackdrop | undefined,
  show: (open: boolean, context?: T) => void,
) {
  let root: MirrorElement | null = null;
  let level: Level | null = null;

  function present() {
    if (!root || !level) return;
    level.root = root;
    level.nativeId = root.__skId;
    const { anchor, point } = level.at;
    __sk_context_menu_open(root.__skId, anchor.__skId, point?.x, point?.y, levels.indexOf(level) > 0, level.backdrop);
    if (level.keyboard) setActive(level, items(level)[0] ?? null, true);
  }

  function close() {
    if (level) closeFrom(levels.indexOf(level));
  }

  function start(at: Location, context: T | undefined, parent: Level | undefined, keyboard: boolean) {
    closeFrom(parent ? levels.indexOf(parent) + 1 : 0);
    if (!levels.length) listen(true);
    level = {
      root,
      nativeId: 0,
      at,
      active: null,
      keyboard,
      backdrop: backdrop() ?? parent?.backdrop ?? "acrylic",
      hide() {
        level = null;
        show(false);
      },
    };
    levels.push(level);
    show(true, context);
    present();
  }

  return {
    open(from: SkMenuAnchor, context?: T) {
      const at = location(from);
      if (at) start(at, context, undefined, !at.point);
    },
    openSubmenu(trigger: Element, keyboard: boolean) {
      const anchor = element(trigger);
      const parent = levelFor(anchor);
      if (anchor && parent && !levels.some((entry) => entry.at.anchor === anchor))
        start({ anchor }, undefined, parent, keyboard);
    },
    close,
    attach(node: HTMLElement | null) {
      const el = element(node);
      if (el === root) return;
      root?.removeEventListener("mouseover", highlight);
      root?.removeEventListener("mouseleave", cancelHover);
      root = el;
      if (root) {
        root.addEventListener("mouseover", highlight);
        root.addEventListener("mouseleave", cancelHover);
        present();
      }
    },
  };
}

export function bindSubmenu(el: HTMLElement, open: (keyboard: boolean) => void): () => void {
  const trigger = element(el);
  if (trigger) submenus.set(trigger, open);
  return () => {
    if (trigger) submenus.delete(trigger);
  };
}

export function selectMenuItem(e: MouseEvent, disabled: boolean, select: () => void) {
  if (disabled) return;
  const item = element(e.currentTarget);
  if (!levelFor(item)) return;
  const current = levels[0];
  try {
    select();
  } finally {
    if (levels[0] === current) closeFrom(0);
  }
}

/** @internal Both renderers share one stack of native menu windows per JS realm. */
export const contextMenuRenderer = { createContextMenu, bindSubmenu, selectMenuItem };

function dismiss(root: number) {
  closeFrom(levels.findIndex((level) => level.nativeId === root));
}
