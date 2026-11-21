import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/focus-press/src/main.ts",
  window: [520, 420],
  label: "focus-press",
});
afterAll(async () => {
  await app.close();
});

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

test(
  "a focus handler can position the native caret before the next edit",
  async () => {
    await app.eval(`(() => {
      const field = document.getElementById("field");
      document.getElementById("btn").focus();
      field.value = "42";
      field.addEventListener("focus", e => e.target.setSelectionRange(1, 1), { once: true });
      field.focus();
    })()`);
    const insert = (text: string) => app.send("ime", { selector: "#field", text, commit: true });
    const fieldValue = () => app.eval('document.getElementById("field").value');
    await insert("7");
    bunExpect(await fieldValue()).toBe("472");

    await app.eval(`(() => {
      const field = document.getElementById("field");
      field.value = "я🙂z";
      field.setSelectionRange(1, 3, "backward");
    })()`);
    await insert("X");
    bunExpect(await fieldValue()).toBe("яXz");
  },
  T,
);

type Result = { log: string[]; active: string };

async function press(id: string, button: 0 | 2 = 0): Promise<Result> {
  bunExpect(await app.eval("__SK_TEST_PRESS__.reset()")).toBe("field");
  await app.locator(`#${id}`).click({ real: true, button });
  // The mouse-up's events are delivered once gpui's dispatch is over.
  await sleep(150);
  return JSON.parse(
    String(await app.eval("JSON.stringify({ log: __SK_TEST_PRESS__.log, active: __SK_TEST_PRESS__.active() })")),
  ) as Result;
}

function chrome(target: string, to: string | null | "kept", last: "click" | "contextmenu" = "click"): string[] {
  const down = [`pointerdown:${target} active=field`, `mousedown:${target} active=field`];
  const now = to === "kept" ? "field" : (to ?? "BODY");
  const moved =
    to === "kept"
      ? []
      : [
          `blur:field rel=${to} active=BODY`,
          `focusout:field rel=${to} active=BODY`,
          ...(to ? [`focus:${to} rel=field active=${to}`, `focusin:${to} rel=field active=${to}`] : []),
        ];
  return [
    ...down,
    ...moved,
    `pointerup:${target} active=${now}`,
    `mouseup:${target} active=${now}`,
    `${last}:${target} active=${now}`,
  ];
}

describe("a press focuses the nearest focusable area, before its mouseup", () => {
  const left: [string, string | null | "kept", string][] = [
    ["tag", null, "a <span> with a focus listener is not focusable"],
    ["keys", null, "nor is a <div> with key listeners"],
    ["plain", null, "a plain <div> blurs to body"],
    ["nolink", null, "an <a> without href is not focusable"],
    ["role-btn", null, "role=button without tabindex is not focusable"],
    ["minus", "minus", "tabindex=-1 takes a press's focus"],
    ["inner", "outer", "a press on an inner <span> focuses its focusable ancestor"],
    ["btn", "btn", "a <button>"],
    ["link", "link", "an <a href>"],
    ["ce", "ce", "contenteditable"],
    ["sum", "sum", "a <details>'s <summary>"],
    ["prevent", "kept", "a cancelled mousedown moves nothing"],
    ["steal", "steal", "a mousedown that focuses the focused field changes nothing"],
  ];
  for (const [id, to, what] of left) {
    test(
      `left press on #${id}: ${what}`,
      async () => {
        const { log, active } = await press(id);
        bunExpect(log).toEqual(chrome(id, to));
        bunExpect(active).toBe(to === "kept" ? "field" : (to ?? "BODY"));
      },
      T,
    );
  }

  test(
    "a mousedown handler's focus() is overridden by the press's own",
    async () => {
      const { log, active } = await press("steal2");
      bunExpect(log).toEqual([
        "pointerdown:steal2 active=field",
        "mousedown:steal2 active=field",
        "blur:field rel=btn active=BODY",
        "focusout:field rel=btn active=BODY",
        "focus:btn rel=field active=btn",
        "focusin:btn rel=field active=btn",
        "blur:btn rel=steal2 active=BODY",
        "focusout:btn rel=steal2 active=BODY",
        "focus:steal2 rel=btn active=steal2",
        "focusin:steal2 rel=btn active=steal2",
        "pointerup:steal2 active=steal2",
        "mouseup:steal2 active=steal2",
        "click:steal2 active=steal2",
      ]);
      bunExpect(active).toBe("steal2");
    },
    T,
  );

  test(
    "a target its mousedown's microtask removes gives the focus to its focusable ancestor, or to body",
    async () => {
      try {
        const gone = await press("gone");
        bunExpect(gone.log.slice(0, 5)).toEqual([
          "pointerdown:gone active=field",
          "mousedown:gone active=field",
          "removed:gone",
          "blur:field rel=null active=BODY",
          "focusout:field rel=null active=BODY",
        ]);
        bunExpect(gone.log.filter((l) => l.startsWith("focus:"))).toEqual([]);
        bunExpect(gone.active).toBe("BODY");

        const gone2 = await press("gone2");
        bunExpect(gone2.log.slice(0, 7)).toEqual([
          "pointerdown:gone2 active=field",
          "mousedown:gone2 active=field",
          "removed:gone2",
          "blur:field rel=wrapper active=BODY",
          "focusout:field rel=wrapper active=BODY",
          "focus:wrapper rel=field active=wrapper",
          "focusin:wrapper rel=field active=wrapper",
        ]);
        bunExpect(gone2.active).toBe("wrapper");
      } finally {
        await app.eval("__SK_TEST_PRESS__.restore()");
      }
    },
    T,
  );
});

describe("a right press focuses before its contextmenu", () => {
  const right: [string, string | null, string[]][] = [
    ["btn", "btn", []],
    ["minus", "minus", []],
    ["menu-row", null, ["row-menu"]],
    ["tag", null, []],
  ];
  for (const [id, to, after] of right) {
    test(
      `right press on #${id}`,
      async () => {
        const { log, active } = await press(id, 2);
        bunExpect(log).toEqual([...chrome(id, to, "contextmenu"), ...after]);
        bunExpect(active).toBe(to ?? "BODY");
      },
      T,
    );
  }
});

describe("what a listener does not make focusable (FO-3)", () => {
  test(
    "focus() takes what Chrome takes",
    async () => {
      bunExpect(JSON.parse(String(await app.eval("__SK_TEST_PRESS__.programmatic()")))).toEqual({
        tag: "field",
        keys: "field",
        minus: "minus",
        plain: "field",
        nolink: "field",
        link: "link",
        ce: "ce",
        sum: "sum",
        "role-btn": "field",
        lbl: "field",
      });
    },
    T,
  );

  test(
    "after a press on a <div> with a keydown listener, a key goes to the document, not to it",
    async () => {
      const { active } = await press("keys");
      bunExpect(active).toBe("BODY");
      await app.press("x");
      await sleep(100);
      bunExpect(await app.eval("__SK_TEST_PRESS__.log.includes('keys-listener')")).toBe(false);
    },
    T,
  );
});
