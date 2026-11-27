import type { MirrorElement } from "./runtime/types";

export type SkMenuBackdrop = "acrylic" | "mica" | "mica-alt" | "transparent" | "none";
export type SkMenuAnchor = Element | MouseEvent;

export interface SkContextMenuHandle<T = unknown> {
  open(anchor: SkMenuAnchor, context?: T): void;
  close(): void;
}

type Point = { x: number; y: number };
type Location = { anchor: MirrorElement; point?: Point };
const enum PointerAt {
  Trigger,
  Row,
  Gap,
  Reset,
}
type Level = {
  root: MirrorElement | null;
  nativeId: number;
  at: Location;
  backdrop: SkMenuBackdrop;
  active: MirrorElement | null;
  pointer: Point | null;
  parent: Level | undefined;
  debugAim: boolean;
  keyboard: boolean;
  hide(): void;
};
const levels: Level[] = [];
const submenus = new WeakMap<MirrorElement, (keyboard: boolean) => void>();
let pendingHover: { level: Level; item: MirrorElement; timer: ReturnType<typeof setTimeout> } | undefined;

function cancelHover(level?: Level) {
  if (!pendingHover || (level && pendingHover.level !== level)) return;
  clearTimeout(pendingHover.timer);
  pendingHover = undefined;
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

function menuPointer(child: Level, p: Point | null, at: PointerAt): boolean {
  return (
    !!child.nativeId &&
    typeof __sk_context_menu_pointer === "function" &&
    __sk_context_menu_pointer(child.nativeId, p?.x ?? 0, p?.y ?? 0, at)
  );
}

function highlight(e: Event) {
  const target = element(e.target);
  const level = levelFor(target);
  if (!level || level.root !== element(e.currentTarget)) return;
  const { clientX: x, clientY: y } = e as MouseEvent;
  level.pointer = { x, y };
  if (level.parent) menuPointer(level, null, PointerAt.Reset);
  const i = levels.indexOf(level);
  const child = levels[i + 1];
  const item = target?.closest('[role="menuitem"]');
  if (child) {
    if (item === child.at.anchor) {
      menuPointer(child, level.pointer, PointerAt.Trigger);
      setActive(level, item);
      return;
    }
    if (menuPointer(child, level.pointer, item ? PointerAt.Row : PointerAt.Gap)) return;
  }
  if (!item) {
    cancelHover(level);
    return;
  }
  const enabled = item.getAttribute("aria-disabled") !== "true";
  const open = enabled ? submenus.get(item) : undefined;
  setActive(level, enabled ? item : null);
  if (child) {
    closeFrom(i + 1);
    open?.(false);
  } else if (!open) {
    cancelHover(level);
  } else if (pendingHover?.item !== item) {
    cancelHover();
    pendingHover = {
      level,
      item,
      timer: setTimeout(() => {
        pendingHover = undefined;
        if (level.active === item && item.isConnected) open(false);
      }, 160),
    };
  }
}

function leave(e: Event) {
  const level = levelFor(element(e.currentTarget));
  if (!level) return;
  level.pointer = null;
  cancelHover(level);
}

function pointerDown(e: Event) {
  const level = levelFor(element(e.target));
  if (!level) return closeFrom(0);
  const { clientX: x, clientY: y } = e as MouseEvent;
  level.pointer = { x, y };
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
  for (const entry of levels) {
    if (entry.parent) menuPointer(entry, null, PointerAt.Reset);
    entry.pointer = null;
  }
  if (next) setActive(level, next, true);
  e.preventDefault();
  e.stopImmediatePropagation();
}

function listen(on: boolean) {
  if (on) {
    Object.assign(globalThis, { __sk_context_menu_dismiss: dismiss });
    document.addEventListener("keydown", keydown, true);
    document.addEventListener("mousedown", pointerDown, true);
  } else {
    document.removeEventListener("keydown", keydown, true);
    document.removeEventListener("mousedown", pointerDown, true);
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
  debugAim?: () => boolean,
) {
  let root: MirrorElement | null = null;
  let level: Level | null = null;

  function present() {
    if (!root || !level) return;
    level.root = root;
    level.nativeId = root.__skId;
    const { anchor, point } = level.at;
    __sk_context_menu_open(
      root.__skId,
      anchor.__skId,
      point?.x,
      point?.y,
      !!level.parent,
      level.backdrop,
      level.parent?.nativeId ?? 0,
      level.debugAim,
    );
    if (level.parent?.pointer && !level.keyboard) menuPointer(level, level.parent.pointer, PointerAt.Trigger);
    if (level.keyboard) setActive(level, items(level)[0] ?? null, true);
  }

  function close() {
    if (level) closeFrom(levels.indexOf(level));
  }

  function start(at: Location, context: T | undefined, parent: Level | undefined, keyboard: boolean) {
    closeFrom(parent ? levels.indexOf(parent) + 1 : 0);
    if (!levels.length) listen(true);
    if (parent) setActive(parent, at.anchor, keyboard);
    level = {
      root,
      nativeId: 0,
      at,
      active: null,
      pointer: null,
      parent,
      debugAim: parent?.debugAim ?? debugAim?.() ?? false,
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
      root?.removeEventListener("mousemove", highlight);
      root?.removeEventListener("mouseleave", leave);
      root = el;
      if (root) {
        root.addEventListener("mouseover", highlight);
        root.addEventListener("mousemove", highlight);
        root.addEventListener("mouseleave", leave);
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
