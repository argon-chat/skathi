import { createApp, h } from "vue-native";
import "./layout.css";

const range = (n: number) => Array.from({ length: n }, (_, i) => i + 1);
const two = (i: number) => (i < 10 ? `0${i}` : `${i}`);

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

const track = () =>
  h("div", { id: "track", class: "track" }, [
    h("div", { id: "thumb", class: "thumb", style: "left: calc(38.89% + 13.33px)" }),
    h("div", { id: "calc-w", class: "calc-w" }),
  ]);

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

/** The images are SVG data URLs (layout.css), so the fixture carries no binary. */
const backgrounds = () =>
  h("div", { id: "bgs", class: "bgs" }, [
    h("div", { id: "bg-px", class: "bg bg-sheet bg-px" }),
    h("div", { id: "bg-pct", class: "bg bg-sheet bg-pct" }),
    h("div", { id: "bg-cover", class: "bg bg-cover" }),
    h("div", { id: "bg-tile", class: "bg bg-tile" }),
    h("div", { id: "bg-over", class: "bg bg-over" }),
  ]);

const backdrop = () =>
  h("div", { id: "bd-stage", class: "bd-stage" }, [
    h("div", { class: "bd-move" }, [h("div", { id: "bd-glass", class: "bd-glass" })]),
  ]);

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

const hiddens = () =>
  h("div", { class: "hid-at" }, [
    h("div", { id: "hid", class: "hid-box", hidden: true }),
    h("div", { id: "hid-found", class: "hid-box", hidden: "until-found" }),
    h("div", { id: "hid-flex", class: "hid-box hid-flex", hidden: true }),
  ]);

/** Before the fix the mask was dropped and the lime covered the card's text. */
const maskRing = () =>
  h("div", { id: "ring-card", class: "ring-card" }, [
    h("div", { class: "ring-border" }),
    h("span", { class: "ring-text" }, "ANNUAL"),
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
      maskRing(),
    ]),
}).mount();
