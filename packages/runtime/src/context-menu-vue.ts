import { defineComponent, h, mergeProps, onBeforeUnmount, ref, shallowRef, watch } from "@vue/runtime-core";
import type { PropType, VNodeRef } from "@vue/runtime-core";
import { bindSubmenu, createContextMenu, selectMenuItem, type SkMenuBackdrop } from "./context-menu";

const backdropProp = { type: String as PropType<SkMenuBackdrop>, default: undefined };
const preventFocus = (event: MouseEvent) => event.preventDefault();

export const SkContextMenu = /*@__PURE__*/ defineComponent({
  name: "SkContextMenu",
  inheritAttrs: false,
  props: { backdrop: backdropProp },
  emits: ["close"],
  setup(props, { attrs, slots, expose, emit }) {
    const visible = ref(false);
    const context = shallowRef<unknown>();
    const menu = createContextMenu<unknown>(
      () => props.backdrop,
      (open, value) => {
        visible.value = open;
        context.value = value;
        if (!open) emit("close");
      },
    );
    const attach: VNodeRef = (el) => menu.attach(el as HTMLElement | null);
    expose({ open: menu.open, close: menu.close });
    onBeforeUnmount(menu.close);
    return () =>
      visible.value
        ? h(
            "div",
            mergeProps(attrs, {
              class: "sk-context-menu",
              role: "menu",
              ref: attach,
              onMousedown: preventFocus,
            }),
            slots.default?.({ context: context.value }),
          )
        : null;
  },
});

export const SkMenuItem = /*@__PURE__*/ defineComponent({
  name: "SkMenuItem",
  props: { disabled: Boolean },
  emits: ["select"],
  setup(props, { slots, emit }) {
    return () =>
      h(
        "div",
        {
          class: "sk-menu-item",
          role: "menuitem",
          "aria-disabled": String(props.disabled),
          onClick: (event: MouseEvent) => selectMenuItem(event, props.disabled, () => emit("select", event)),
        },
        slots.default?.(),
      );
  },
});

export const SkMenuSeparator = /*@__PURE__*/ defineComponent({
  name: "SkMenuSeparator",
  setup: () => () => h("div", { class: "sk-menu-separator", role: "separator" }),
});

export const SkSubmenu = /*@__PURE__*/ defineComponent({
  name: "SkSubmenu",
  inheritAttrs: false,
  props: { label: { type: String, required: true }, disabled: Boolean, backdrop: backdropProp },
  setup(props, { slots, attrs }) {
    const trigger = shallowRef<HTMLElement | null>(null);
    const visible = ref(false);
    const menu = createContextMenu(
      () => props.backdrop,
      (open) => {
        visible.value = open;
      },
    );
    function open(keyboard: boolean) {
      if (trigger.value && !props.disabled) menu.openSubmenu(trigger.value, keyboard);
    }
    const attach: VNodeRef = (el) => menu.attach(el as HTMLElement | null);
    watch(
      trigger,
      (el, _, cleanup) => {
        if (el) cleanup(bindSubmenu(el, open));
      },
      { flush: "sync" },
    );
    onBeforeUnmount(menu.close);
    return () => [
      h(
        "div",
        mergeProps(attrs, {
          ref: trigger,
          class: "sk-menu-item",
          role: "menuitem",
          "aria-haspopup": "menu",
          "aria-expanded": String(visible.value),
          "aria-disabled": String(props.disabled),
          onClick: () => open(false),
        }),
        [h("span", slots.label?.() ?? props.label), h("span", { class: "sk-menu-arrow", "aria-hidden": "true" })],
      ),
      visible.value
        ? h(
            "div",
            {
              class: "sk-context-menu",
              role: "menu",
              "aria-label": props.label,
              ref: attach,
              onMousedown: preventFocus,
            },
            slots.default?.(),
          )
        : null,
    ];
  },
});
