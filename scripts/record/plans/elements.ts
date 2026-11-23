// The clips of docs/pages/elements, from apps/elements. Each plan returns the loop window: two
// moments with the same picture, so the clip loops without a jump. Continuous animations there
// have periods that divide the window (Lottie 3 s, stickers 2 s, chart and surface 8 s, video 9 s
// plus the time it was paused, the surface example's plasma layer 4 s).
import { Driver, until } from "../driver";
import type { AppPlans } from "../record";
import { sleep } from "../win32";

const still = (ms: number) => async () => {
  const start = Date.now();
  await until(start, ms + 300);
  return { start, end: start + ms };
};

export const plans: AppPlans = {
  app: "elements",
  group: "elements",
  window: [1280, 800],
  clips: {
    lottie: {
      poster: 0.02,
      async plan(d: Driver) {
        const btn = await d.center("[data-demo=toggle]");
        const rest = { x: btn.x + 190, y: btn.y - 120 };
        const start = Date.now();
        await until(start, 1500);
        await d.show(rest);
        await d.move(btn, 650);
        await d.click();
        const paused = Date.now();
        await sleep(1600);
        await d.click();
        const resumed = Date.now();
        await sleep(250);
        await d.move(rest, 650);
        await sleep(150);
        await d.hide();
        const end = start + 9000 + (resumed - paused);
        await until(end, 300);
        return { start, end };
      },
    },

    "sk-sticker": { poster: 0.4, plan: still(8000) },

    video: {
      poster: 0.3,
      settle: 2500,
      async plan(d) {
        const frame = await d.box("[data-demo=frame]");
        const outside = { x: Math.round(frame.x + frame.w + 60), y: Math.round(frame.y + frame.h - 150) };
        const start = Date.now();
        await until(start, 1100);
        await d.show(outside);
        const scrub = await d.box("[data-demo=scrub]");
        await d.move({ x: Math.round(frame.x + frame.w * 0.62), y: Math.round(frame.y + frame.h * 0.6) }, 700);
        await sleep(300);
        await d.move({ x: Math.round(scrub.x + scrub.w * 0.55), y: Math.round(scrub.y + scrub.h / 2) }, 600);
        await sleep(700);
        await d.click(await d.center("[data-demo=play]"));
        const paused = Date.now();
        await sleep(1500);
        await d.click();
        const resumed = Date.now();
        await sleep(500);
        await d.move(outside, 800);
        await sleep(150);
        await d.hide();
        const end = start + 9000 + (resumed - paused);
        await until(end, 300);
        return { start, end };
      },
    },

    canvas: {
      poster: 0.45,
      async plan(d) {
        const c = await d.box("[data-demo=chart]");
        const outside = { x: Math.round(c.x + c.w - 40), y: Math.round(c.y + c.h + 70) };
        const start = Date.now();
        await until(start, 1200);
        await d.show(outside);
        const a = { x: c.x + c.w * 0.3, y: c.y + c.h * 0.45 };
        const b = { x: c.x + c.w * 0.82, y: c.y + c.h * 0.35 };
        await d.move({ x: Math.round(a.x), y: Math.round(a.y) }, 800);
        const from = Date.now();
        while (Date.now() - from < 2600) {
          const t = (Date.now() - from) / 2600;
          const e = t * t * (3 - 2 * t);
          await d.jump({ x: Math.round(a.x + (b.x - a.x) * e), y: Math.round(a.y + (b.y - a.y) * e) });
          await sleep(10);
        }
        await sleep(400);
        await d.move(outside, 800);
        await sleep(150);
        await d.hide();
        await until(start, 8300);
        return { start, end: start + 8000 };
      },
    },

    "sk-surface": { poster: 0.3, host: "surface", plan: still(8000) },

    "sk-editor": {
      poster: 0.76,
      async plan(d) {
        const field = await d.box("[data-demo=editor]");
        const rest = { x: Math.round(field.x + field.w - 60), y: Math.round(field.y + field.h + 70) };
        const ed = `document.querySelector("[data-demo=editor]")`;
        const start = Date.now();
        await until(start, 500);
        await d.show(rest);
        await d.click({ x: Math.round(field.x + field.w * 0.6), y: Math.round(field.y + field.h / 2) });
        await sleep(150);
        await d.type("[data-demo=editor]", "Ship the new palette, ", 55);
        await sleep(200);
        await d.click(await d.center("[data-demo=mention]"));
        await sleep(250);
        await d.type("[data-demo=editor]", "can you check it by Fryday?", 55);
        await sleep(500);
        const raw = await d.js(
          `JSON.stringify((() => { const e = ${ed}; const t = e.slice(0); const f = t.indexOf("new palette"); return e.rectsFor(f, f + 11).map((r) => [r.x, r.y, r.width, r.height]); })())`,
        );
        const [[x, y, w, h]] = JSON.parse(String(raw)) as [[number, number, number, number]];
        await d.move({ x: Math.round(x + 1), y: Math.round(y + h / 2) }, 600);
        await sleep(100);
        await d.down();
        await d.move({ x: Math.round(x + w - 1), y: Math.round(y + h / 2) }, 500);
        await d.up();
        await sleep(250);
        await d.click(await d.center("[data-demo=bold]"));
        await sleep(200);
        await d.click(await d.center("[data-demo=accent]"));
        await sleep(350);
        await d.move({ x: Math.round(field.x + field.w - 40), y: Math.round(field.y + field.h / 2) }, 500);
        await d.badge("End");
        await sleep(260);
        await d.app.send("key", { key: "End", selector: "[data-demo=editor]" });
        await sleep(350);
        await d.badge("");
        await sleep(1300);
        await d.badge("Ctrl+Z");
        await sleep(260);
        for (let i = 0; i < 20 && String(await d.js(`${ed}.length`)) !== "0"; i++) {
          await d.app.send("key", { key: "Ctrl+Z", selector: "[data-demo=editor]" });
          await sleep(170);
        }
        await sleep(250);
        await d.badge("");
        await sleep(200);
        await d.click(rest);
        await sleep(150);
        await d.hide();
        const end = Date.now() + 500;
        await until(end, 300);
        return { start, end };
      },
    },

    "sk-editor-code": {
      poster: 0.45,
      settle: 2500,
      async plan(d) {
        const code = await d.box("[data-demo=code]");
        const find = await d.center("[data-demo=find]");
        const ed = `document.querySelector("[data-demo=code]")`;
        const rest = { x: Math.round(code.x + code.w - 120), y: Math.round(code.y + code.h - 60) };
        const start = Date.now();
        await until(start, 500);
        await d.show(rest);
        // The long line: its end runs past the edge.
        const raw = await d.js(
          `JSON.stringify((() => { const e = ${ed}; const l = e.line(11); const r = e.rectsFor(l.from + 20, l.from + 21)[0]; return [r.x, r.y, r.height]; })())`,
        );
        const [lx, ly, lh] = JSON.parse(String(raw)) as [number, number, number];
        await d.click({ x: Math.round(lx + 120), y: Math.round(ly + lh / 2) });
        await sleep(500);
        await d.badge("Shift+Wheel");
        for (let i = 0; i < 14; i++) {
          await d.wheel(0, 60, true);
          await sleep(40);
        }
        await sleep(700);
        for (let i = 0; i < 14; i++) {
          await d.wheel(0, -60, true);
          await sleep(40);
        }
        await d.badge("");
        await sleep(300);
        await d.click(find);
        await d.type("[data-demo=find]", "frame", 90);
        await sleep(700);
        await d.move({ x: Math.round(code.x + code.w * 0.55), y: Math.round(code.y + code.h * 0.5) }, 600);
        await d.badge("Wheel");
        for (let i = 0; i < 45; i++) {
          await d.wheel(0, 150 + i * 80);
          await sleep(30);
        }
        await d.badge("");
        await sleep(300);
        for (const key of ["Ctrl+End", "Ctrl+Home"]) {
          await d.badge(key);
          await sleep(260);
          await d.app.send("key", { key, selector: "[data-demo=code]" });
          await sleep(1100);
          await d.badge("");
          await sleep(300);
        }
        await d.click(find);
        await d.key("Escape", "Esc", 400);
        await d.click(await d.center("[data-demo=status]", -220));
        await sleep(150);
        await d.hide();
        const end = Date.now() + 500;
        await until(end, 300);
        return { start, end };
      },
    },

    "sk-layer": {
      poster: 0.06,
      host: "surface",
      async plan(d) {
        const bar = await d.box(".bar");
        const rest = { x: Math.round(bar.x + bar.w + 120), y: Math.round(bar.y + bar.h / 2 - 50) };
        const start = Date.now();
        await until(start, 900);
        await d.show(rest);
        await d.click(await d.center("[data-demo=round]"));
        await sleep(2000);
        await d.click(await d.center("[data-demo=hidden]"));
        await sleep(1900);
        await d.click(await d.center("[data-demo=wide]"));
        await sleep(1300);
        await d.move(rest, 650);
        await sleep(100);
        await d.hide();
        const end = start + 12000;
        await until(end, 300);
        return { start, end };
      },
    },

    iframe: {
      poster: 0.3,
      settle: 6000,
      async plan(d) {
        const panel = await d.box("[data-demo=panel]");
        const rest = { x: Math.round(panel.x - 40), y: Math.round(panel.y + panel.h + 110) };
        const start = Date.now();
        await until(start, 900);
        await d.show(rest);
        await d.click(await d.center("[data-demo=accent-lilac]"));
        await sleep(1700);
        await d.click(await d.center("[data-demo=accent-blue]"));
        await sleep(1700);
        await d.click(await d.center("[data-demo=accent-pink]"));
        const last = Date.now();
        await sleep(300);
        await d.move(rest, 700);
        await sleep(150);
        await d.hide();
        const end = last + 2300;
        await until(end, 300);
        return { start, end };
      },
    },

    "text-fields": {
      poster: 0.38,
      async plan(d) {
        // Clicked on the right, so the pointer stays clear of the text.
        const title = await d.center("[data-demo=title]", 170);
        const notes = await d.box("[data-demo=notes]");
        const rest = { x: title.x + 110, y: title.y + 300 };
        const value = () => d.js(`document.querySelector("[data-demo=title]").value`);
        const start = Date.now();
        await until(start, 500);
        await d.show(rest);
        await d.click(title);
        await sleep(150);
        await d.type("[data-demo=title]", "Picnic at dusk", 45);
        await sleep(250);
        await d.key("Ctrl+Shift+Left", "Ctrl+Shift+←", 550);
        await d.type("[data-demo=title]", "sunset", 50);
        await sleep(350);
        // An undo step is one character: repeat it under one badge, like a held key.
        await d.badge("Ctrl+Z");
        await sleep(260);
        for (let i = 0; i < 12 && (await value()) !== "Picnic at dusk"; i++) {
          await d.app.send("key", { key: "Ctrl+Z" });
          await sleep(95);
        }
        await sleep(300);
        await d.badge("");
        await sleep(250);
        await d.click({ x: Math.round(notes.x + notes.w - 120), y: Math.round(notes.y + notes.h - 30) });
        await sleep(150);
        await d.type("[data-demo=notes]", "Lilac blanket\nPink lemonade", 35);
        await sleep(500);
        await d.click(await d.center("[data-demo=cancel]"));
        await sleep(250);
        await d.move(rest, 650);
        await sleep(100);
        await d.hide();
        const end = Date.now() + 450;
        await until(end, 300);
        return { start, end };
      },
    },

    "form-controls": {
      poster: 0.62,
      async plan(d) {
        const rest = await d.center("[data-demo=reset]", 150, 70);
        const start = Date.now();
        await until(start, 400);
        await d.show(rest);
        await d.click(await d.center("[data-demo=autoplay] input"));
        await sleep(150);
        await d.click(await d.center("[data-demo=q-1080p] input"));
        await sleep(150);
        const r = await d.box("[data-demo=volume]");
        const at = (v: number) => ({ x: Math.round(r.x + 8 + (v / 100) * (r.w - 16)), y: Math.round(r.y + r.h / 2) });
        await d.move(at(40), 500);
        await sleep(80);
        await d.down();
        await d.move(at(82), 850);
        await sleep(80);
        await d.up();
        await sleep(200);
        await d.click(await d.center("[data-demo=preset-9fcdf5]"));
        await sleep(300);
        const sel = await d.box("[data-demo=subtitles]");
        await d.click({ x: Math.round(sel.x + sel.w / 2), y: Math.round(sel.y + sel.h / 2) });
        await sleep(400);
        // The list is drawn by the host under the box, 19 px a row; "English" is the second.
        await d.move({ x: Math.round(sel.x + sel.w / 2 - 30), y: Math.round(sel.y + sel.h + 19 * 1.5) }, 420);
        await sleep(150);
        await d.click();
        await sleep(400);
        await d.click(await d.center("[data-demo=reset]"));
        await sleep(250);
        await d.move(rest, 550);
        await sleep(100);
        await d.hide();
        const end = Date.now() + 500;
        await until(end, 300);
        return { start, end };
      },
    },
  },
};
