import { describe, expect, test } from "bun:test";
import { resolve } from "node:path";
import { nativeSvelteRenderer } from "../cli/config";
import { compileSvelte, SVELTE_RENDERER } from "../svelte";

const root = process.env.SK_SVELTE_TEST_ROOT ?? resolve(import.meta.dir, "../../..");
const compile = (source: string, native: boolean) =>
  compileSvelte(root, "Component.svelte", source, { filename: "Component.svelte", dev: false, native });

describe("Svelte custom renderer compatibility", () => {
  test("native rendering requires an explicit config choice", () => {
    expect(nativeSvelteRenderer(undefined)).toBe(false);
    expect(nativeSvelteRenderer({ renderer: "dom" })).toBe(false);
    expect(nativeSvelteRenderer({ renderer: "native" })).toBe(true);
    expect(() => nativeSvelteRenderer({ renderer: "server" })).toThrow("svelte.renderer");
    expect(() => nativeSvelteRenderer({ render: "native" })).toThrow("svelte.render");
  });

  test.each([
    ["input binding", '<script>let x=$state("");</script><input bind:value={x}/>'],
    ["select binding", '<script>let x=$state("");</script><select bind:value={x}><option>hi</option></select>'],
    ["checkbox binding", "<script>let b=$state(false);</script><input type=checkbox bind:checked={b}/>"],
    ["radio group", '<script>let value=$state("a");</script><input type=radio value=a bind:group={value}/>'],
    ["binding a DOM reference", "<script>let el=$state();</script><div bind:this={el}></div>"],
    ["dimension binding", "<script>let w=$state(0);</script><div bind:clientWidth={w}></div>"],
    ["intro/outro", '<script>import {fade} from "svelte/transition";</script><div transition:fade></div>'],
    [
      "FLIP",
      '<script>import {flip} from "svelte/animate";</script>{#each [1] as n (n)}<div animate:flip>{n}</div>{/each}',
    ],
    ["legacy event", "<button on:click={() => {}}>Play</button>"],
    ["injected CSS", '<svelte:options css="injected"/><p>Hello</p><style>p{color:red}</style>'],
    ...["window", "document", "body", "head"].map((name) => [name, `<svelte:${name}></svelte:${name}>`]),
  ])("%s stays available with the default renderer", async (_, source) => {
    expect((await compile(source!, false)).js).not.toContain(SVELTE_RENDERER);
    await expect(compile(source!, true)).rejects.toMatchObject({ code: "SK5015", file: "Component.svelte" });
  });

  test.each([
    "<script>let x=$state(1); let y=$derived(x*2); $effect(()=>y)</script><button onclick={()=>x++}>{y}</button>",
    "{#snippet row(i)}<p>{i}</p>{/snippet}{#each [1,2] as i (i)}{@render row(i)}{/each}",
    "{#await Promise.resolve(1) then x}{#key x}{#if x}{x}{/if}{/key}{/await}",
    '<script>import {writable} from "svelte/store"; const n=writable(1);</script><p>{$n}</p>',
    '<svelte:element this={"article"} class:active={true} style:color={"red"} />',
    "<svelte:boundary><div {@attach el=>{void el}}></div></svelte:boundary>",
    "<script>const useIt=el=>({destroy(){}});</script><div use:useIt></div>",
  ])("compiles supported native markup %#", async (source) => {
    expect((await compile(source, true)).js).toContain(SVELTE_RENDERER);
    expect((await compile(source, false)).js).not.toContain(SVELTE_RENDERER);
  });
});
