// A fixture for packages/vn-test/specs/layout.spec.ts — the m26 client sweep's
// layout and paint findings in the Argon client, each reduced to its shape
// (research/reports/m26/client-sweep-1.md, fixes in fixes-layout-1.md).
//
// One section per finding, each laid out where the spec can hit it. The class
// names spell out the Tailwind utilities the client wrote, so a rule maps back
// to `apps/client` (MessageItem.vue, MentionSegment.vue, reka's Slider,
// InventoryView.vue). Render functions rather than an SFC: this package is
// `"type": "module"`, and the dev builder's CommonJS interop hands such an
// importer a `.vue` module's NAMESPACE as its default export.
import { createApp, h } from "vue-native";
import "./layout.css";

const range = (n: number) => Array.from({ length: n }, (_, i) => i + 1);
const two = (i: number) => (i < 10 ? `0${i}` : `${i}`);

/** #1: a chat row's timestamp, `hidden group-hover/msg:flex`, in a scroller. */
const messages = () =>
  h(
    "div",
    { id: "msgs", class: "msgs" },
    range(30).map((i) =>
      h("div", { id: `msg-${i}`, key: i, class: "group-msg msg" }, [
        h("div", { class: "gutter" }, [
          h("span", { id: `ts-${i}`, class: "hidden group-msg-hover-flex ts" }, `12:${two(i)}`),
        ]),
        h("div", { class: "body" }, `message ${i}`),
      ]),
    ),
  );

/** #9: MessageItem's bubble around a mention, nested as the client nests it. */
const bubble = () =>
  h("div", { class: "chat" }, [
    h("div", { class: "m-row" }, [
      h("div", { class: "m-avatar" }),
      h("div", { id: "m-col", class: "m-col" }, [
        h("div", { class: "m-meta" }, "name 12:00"),
        h("span", { id: "m-trigger" }, [
          h("div", { class: "m-rowbody" }, [
            h("div", { id: "m-wrap", class: "m-wrap" }, [
              h(
                "div",
                { id: "m-media", class: "m-media", style: "max-width: calc(var(--chat-width, 600px) * 0.8)" },
                [
                  h("div", { id: "m-bubble", class: "m-bubble" }, [
                    h("div", { id: "m-text", class: "m-text" }, [
                      h("button", { id: "m-mention", type: "button" }, [
                        h("span", { class: "mention" }, [
                          h("span", { id: "m-at", class: "mention-icon" }, "@"),
                          h("span", { id: "m-name", class: "mention-name" }, "owner"),
                        ]),
                      ]),
                      " hello there",
                    ]),
                  ]),
                ],
              ),
            ]),
          ]),
        ]),
      ]),
    ]),
  ]);

/** #12: reka Slider's thumb, and a Tailwind `w-[calc(100%-40px)]`. */
const track = () =>
  h("div", { id: "track", class: "track" }, [
    h("div", { id: "thumb", class: "thumb", style: "left: calc(38.89% + 13.33px)" }),
    h("div", { id: "calc-w", class: "calc-w" }),
  ]);

/** #15: `repeat(auto-fill, …)` and `repeat(auto-fit, …)` grids. */
const grids = () => [
  h(
    "div",
    { id: "grid-fill", class: "grid grid-fill" },
    range(7).map((i) => h("div", { id: `fill-${i}`, key: i, class: "cell" }, String(i))),
  ),
  h("div", { id: "grid-fit", class: "grid grid-fit" }, [
    h("div", { id: "fit-1", class: "cell" }, "a"),
    h("div", { id: "fit-2", class: "cell" }, "b"),
  ]),
];

/** #14: `background-image: url()` — emojix's two sprite forms, a cover banner,
 *  a repeat, and a gradient written above an image. The images are SVG data
 *  URLs (layout.css), so the fixture carries no binary. */
const backgrounds = () =>
  h("div", { id: "bgs", class: "bgs" }, [
    h("div", { id: "bg-px", class: "bg bg-sheet bg-px" }),
    h("div", { id: "bg-pct", class: "bg bg-sheet bg-pct" }),
    h("div", { id: "bg-cover", class: "bg bg-cover" }),
    h("div", { id: "bg-tile", class: "bg bg-tile" }),
    h("div", { id: "bg-over", class: "bg bg-over" }),
  ]);

/** #18: a `backdrop-filter` panel inside a box moved by `transform`, the way
 *  a popper is positioned (UserProfilePopover's `.glass-body`), over stripes
 *  a blur turns grey. */
const backdrop = () =>
  h("div", { id: "bd-stage", class: "bd-stage" }, [
    h("div", { class: "bd-move" }, [h("div", { id: "bd-glass", class: "bd-glass" })]),
  ]);

/** #36: @argon/ui's RadioGroupItem, checked. A 16px round `<button>` holds
 *  reka's indicator `<span>` (`relative flex items-center justify-center`),
 *  which holds lucide's CircleIcon at `absolute top-1/2 left-1/2 size-2
 *  -translate-x-1/2 -translate-y-1/2`. */
const radio = () =>
  h("div", { class: "radio-at" }, [
    h("button", { id: "radio", class: "radio", role: "radio", "aria-checked": "true" }, [
      h("span", { class: "radio-ind" }, [
        h(
          "svg",
          {
            id: "radio-dot",
            class: "radio-dot",
            xmlns: "http://www.w3.org/2000/svg",
            width: "24",
            height: "24",
            viewBox: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            "stroke-width": "2",
          },
          [h("circle", { cx: "12", cy: "12", r: "10" })],
        ),
      ]),
    ]),
  ]);

/** #10 (b): the `hidden` attribute, under the UA rule `[hidden] { display:
 *  none }`, which an author `display` beats. */
const hiddens = () =>
  h("div", { class: "hid-at" }, [
    h("div", { id: "hid", class: "hid-box", hidden: true }),
    h("div", { id: "hid-found", class: "hid-box", hidden: "until-found" }),
    h("div", { id: "hid-flex", class: "hid-box hid-flex", hidden: true }),
  ]);

createApp({
  render: () =>
    h("div", { id: "root" }, [
      messages(),
      bubble(),
      track(),
      ...grids(),
      backgrounds(),
      backdrop(),
      radio(),
      hiddens(),
    ]),
}).mount();
