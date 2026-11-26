import { defineComponent, h, mergeProps, onScopeDispose, shallowRef, type PropType } from "@vue/runtime-core";
import { onKeyLayoutChange } from "./keyboard-layout";
import type { SkKeyMap } from "./keymap";
import { kbdPresentation, type SkKbdKeys, type SkKbdLabels, type SkKbdPlatform, type SkKbdProps } from "./kbd";
import type { KbdKeycap } from "./kbd-devices";

function keycap(key: KbdKeycap) {
  const attrs = { class: "sk-kbd-key", "data-device": key.device, "data-shape": key.shape };
  if (!key.glyph) return h("span", attrs, key.text);
  const icon = h(
    "svg",
    {
      class: "sk-kbd-glyph",
      xmlns: "http://www.w3.org/2000/svg",
      viewBox: "0 0 24 24",
      width: 20,
      height: 20,
      fill: "none",
      stroke: "currentColor",
      "stroke-width": 1.6,
      "stroke-linecap": "round",
      "stroke-linejoin": "round",
    },
    [
      h("path", { d: key.glyph.outline }),
      key.glyph.fill && h("path", { d: key.glyph.fill, fill: "currentColor", stroke: "none" }),
    ],
  );
  return h("span", attrs, [icon, h("span", key.text)]);
}

export const SkKbd = /*@__PURE__*/ defineComponent({
  name: "SkKbd",
  inheritAttrs: false,
  props: {
    keys: [String, Array, Object] as PropType<SkKbdKeys | null | undefined>,
    map: Object as PropType<SkKeyMap | null | undefined>,
    platform: String as PropType<SkKbdPlatform>,
    gamepadLayout: String as PropType<SkKbdProps["gamepadLayout"]>,
    mousePrimary: String as PropType<SkKbdProps["mousePrimary"]>,
    variant: { type: String as PropType<SkKbdProps["variant"]>, default: "keycaps" },
    labels: Object as PropType<SkKbdLabels>,
  },
  setup(props, { attrs }) {
    const revision = shallowRef(0);
    onScopeDispose(onKeyLayoutChange(() => revision.value++));
    return () => {
      void revision.value;
      const { groups, label } = kbdPresentation(props);
      if (!groups.length) return null;
      return h(
        "kbd",
        mergeProps(
          {
            class: "sk-kbd",
            role: "group",
            "aria-label": label,
            "data-scope": props.map?.scope,
            "data-variant": props.variant,
          },
          attrs,
        ),
        groups.map((group) =>
          h("span", { class: "sk-kbd-step", "aria-hidden": "true" }, [
            group.before && h("span", { class: "sk-kbd-between" }, group.before),
            ...(props.variant === "plain"
              ? [group.text]
              : group.keys.flatMap((key) => [
                  key.joiner && h("span", { class: "sk-kbd-join" }, key.joiner),
                  keycap(key),
                ])),
          ]),
        ),
      );
    };
  },
});
