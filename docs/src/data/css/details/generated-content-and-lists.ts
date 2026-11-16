import type { Details } from "../../details";

export const details: Details = {
  "pseudo-before": {
    examples: [
      {
        title: "Put a presence dot before a name",
        lang: "css",
        code: `.presence {
  display: flex;
  align-items: center;
  gap: 6px;
}
.presence::before {
  content: "";
  width: 8px;
  height: 8px;
  border-radius: 4px;
  background-color: #9aa0a6;
}
.presence[data-status="online"]::before {
  background-color: #1e8e3e;
}`,
        note: "The dot's colour follows an attribute, because a `:hover` or other state in a pseudo-element selector drops the rule.",
      },
    ],
    edgeCases: [
      "Without a `content` declaration, or with `content: none` or `normal`, no box is generated, whatever its size and background.",
      "A state anywhere in the selector drops the rule, on an ancestor too: `.row:hover .icon::before` is dropped, with an advisory.",
      "`transition` and `animation` do not run on the generated box; animate a real child element instead.",
      "The element's `text-transform` applies to the generated text.",
    ],
  },
  "pseudo-after": {
    examples: [
      {
        title: "Mark required fields",
        lang: "css",
        code: `.field-label.required::after {
  content: " *";
  color: #d93025;
}`,
      },
      {
        title: "Flag links that open in the browser",
        lang: "css",
        code: `a.external::after {
  content: " ↗";
  font-size: 11px;
  color: #5f6368;
}`,
      },
    ],
    edgeCases: [
      "Without a `content` declaration, or with `content: none` or `normal`, no box is generated, whatever its size and background.",
      "A state anywhere in the selector drops the rule, on an ancestor too: `.menu:hover .item::after` is dropped, with an advisory.",
      "`transition` and `animation` do not run on the generated box; animate a real child element instead.",
    ],
  },
  content: {
    examples: [
      {
        title: "Separate breadcrumb items",
        lang: "css",
        code: `.breadcrumbs {
  display: flex;
}
.crumb + .crumb::before {
  content: "/";
  margin: 0px 6px;
  color: #80868b;
}`,
        note: "The separator comes from CSS, so the template renders the items alone.",
      },
    ],
    edgeCases: [
      'Several strings concatenate: `content: "v" "2.1"` renders `v2.1`.',
      "`none` and `normal` generate no box.",
    ],
  },
  "counter-reset": {
    edgeCases: [
      "A `counter-reset` declaration is dropped with an advisory, and `CSS.supports()` returns `false` for it.",
    ],
  },
  "counter-increment": {
    edgeCases: [
      "A `counter-increment` declaration is dropped with an advisory, and `CSS.supports()` returns `false` for it.",
    ],
  },
  "counter-set": {
    edgeCases: [
      "A `counter-set` declaration is dropped with an advisory, and `CSS.supports()` returns `false` for it.",
    ],
  },
  "list-style-type": {
    examples: [
      {
        title: "Number the steps of an onboarding list",
        lang: "css",
        code: `ol.steps {
  list-style-type: decimal;
  padding-left: 24px;
}
ol.steps ol {
  list-style-type: lower-alpha;
}`,
        note: "The nested list declares its own type; without it, its items would take `decimal` from the outer list.",
      },
    ],
    edgeCases: [
      "Lists have no default marker: a `<ul>` or `<ol>` shows none until `list-style-type` or `list-style` is declared on the `<li>` or an ancestor.",
      "A number counts the `<li>`'s position among its parent's element children, from 1. `<ol start>`, `<ol reversed>` and `<li value>` are not read.",
      "Alphabetic markers continue past `z` as `aa`, `ab` and so on, and `decimal-leading-zero` pads to two digits (`07.`).",
      "An unsupported type, such as `hebrew` or a `@counter-style` name, is dropped with an advisory.",
    ],
  },
  "list-style-position": {
    examples: [
      {
        title: "Keep release-note bullets inside the column",
        lang: "css",
        code: `.release-notes {
  list-style-type: disc;
  list-style-position: inside;
  padding-left: 0px;
}`,
        note: "With `inside`, the list needs no left padding for its markers.",
      },
    ],
    edgeCases: [
      "`outside`, the default, puts the marker in a 1.25rem box pulled into the space left of the `<li>`, so the list needs that much left padding or margin to show it.",
      "`inside` puts the same 1.25rem box at the start of the `<li>`, where it takes content width.",
      "It inherits from the nearest ancestor that declares it, as `list-style-type` does.",
    ],
  },
  "list-style": {
    examples: [
      {
        title: "Remove markers from a navigation list",
        lang: "css",
        code: `.nav-list {
  list-style: none;
  margin: 0px;
  padding: 0px;
}`,
      },
      {
        title: "Number appendix sections in Roman numerals",
        lang: "css",
        code: `.appendix {
  list-style: upper-roman inside;
}`,
      },
    ],
    edgeCases: [
      "A shorthand whose type cannot be drawn still sets the position, and gives an advisory for the type.",
      "`list-style: none` removes markers that an ancestor's declaration would otherwise give.",
    ],
  },
  "list-style-image": {
    examples: [
      {
        title: "Instead: draw checkmark bullets",
        lang: "css",
        code: `.checklist li {
  display: flex;
  gap: 8px;
  list-style-type: none;
}
.checklist li::before {
  content: "✓";
  color: #1e8e3e;
}`,
      },
    ],
    edgeCases: ["`list-style-image: url(…)` is dropped with an advisory; `list-style-image: none` is accepted."],
  },
  "pseudo-marker": {
    examples: [
      {
        title: "Instead: colour the bullets of a tips list",
        lang: "css",
        code: `.tips li {
  display: flex;
  gap: 6px;
  list-style-type: none;
}
.tips li::before {
  content: "•";
  color: #1a73e8;
}`,
      },
    ],
    edgeCases: ["A rule with a `::marker` selector is dropped, with an advisory."],
  },
  "object-fit": {
    examples: [
      {
        title: "Crop avatars to a circle",
        lang: "css",
        code: `.avatar {
  width: 40px;
  height: 40px;
  border-radius: 20px;
  object-fit: cover;
}`,
        note: "`cover` fills the square and crops the longer side, so non-square photos are not stretched.",
      },
      {
        title: "Fill a video call tile edge to edge",
        lang: "css",
        code: `.call-tile video {
  width: 100%;
  height: 100%;
  object-fit: cover;
}`,
      },
    ],
    edgeCases: [
      "`<img>` and `<canvas>` default to `fill`, which stretches; `<video>` defaults to `contain`, as in HTML.",
      "`<lottie>` reads it too, and it wins over the element's `objectfit` attribute.",
      "Declared in a state rule such as `:hover`, it applies whatever the state, with an advisory.",
    ],
  },
  "object-position": {
    examples: [
      {
        title: "Keep faces in frame when cropping a cover photo",
        lang: "css",
        code: `.cover-photo {
  width: 100%;
  height: 160px;
  object-fit: cover;
  object-position: center 25%;
}`,
      },
      {
        title: "Pan a banner when its slide is focused",
        lang: "css",
        code: `.banner {
  object-fit: cover;
  object-position: left center;
  transition: object-position 600ms ease-in-out;
}
.banner.focused {
  object-position: right center;
}`,
        note: "`object-position` interpolates, so the image pans across the box instead of jumping.",
      },
    ],
    edgeCases: [
      "A percentage is a share of the free space, the box minus the drawn image: `0%` lines up the left edges and `100%` the right edges. An offset such as `right 12px` measures from that edge.",
      "It interpolates in transitions, `@keyframes` and `element.animate()`; from an unset value it starts at the centred default.",
    ],
  },
  "image-rendering": {
    edgeCases: [
      "An `image-rendering` declaration is dropped with an advisory, and `CSS.supports()` returns `false` for it.",
    ],
  },
  "image-orientation": {
    edgeCases: [
      "An `image-orientation` declaration is dropped with an advisory, and `CSS.supports()` returns `false` for it.",
    ],
  },
};
