<script lang="ts">
  import { onMount, setContext, tick } from "svelte";
  import type { Action } from "svelte/action";
  import type { Attachment } from "svelte/attachments";
  import { cubicOut } from "svelte/easing";
  import { Spring, Tween } from "svelte/motion";
  import { MediaQuery, SvelteMap, SvelteSet } from "svelte/reactivity";
  import { derived, writable } from "svelte/store";
  import { fade, fly } from "svelte/transition";
  import { flip } from "svelte/animate";
  import Boom from "./Boom.svelte";
  import Child from "./Child.svelte";
  import LegacyParent from "./LegacyParent.svelte";
  import { log, release } from "./log";

  let { label }: { label: string } = $props();
  let count = $state(0);
  const doubled = $derived(count * 2);
  $effect(() => {
    log(`effect:${count}`);
  });
  setContext("ctx", "from-context");

  let childValue = $state("init");
  let promise = $state(release("await", "awaited"));
  let keyVal = $state(0);
  let show = $state(false);
  const tween = new Tween(0, { duration: 200, easing: cubicOut });
  const spring = new Spring(0);

  let text = $state("");
  let picked = $state("b");
  let many = $state<string[]>([]);
  let agreed = $state(false);
  let mixed = $state(true);
  let fruits = $state<string[]>(["pear"]);
  let size = $state("m");
  let list = $state([1, 2, 3]);
  let el = $state<HTMLElement>();
  let width = $state(0);
  let innerWidth = $state(0);
  let tag = $state("section");
  let boomFail = $state(false);

  const store = writable(1);
  const storeDouble = derived(store, (s) => s * 2);
  const map = new SvelteMap<string, number>();
  const set = new SvelteSet<number>();
  const wide = new MediaQuery("min-width: 600px");

  const tint: Attachment<HTMLElement> = (node) => {
    node.style.color = "rgb(0, 128, 0)";
    log("attach");
  };
  const mark: Action<HTMLElement, string> = (node, value) => {
    node.dataset.mark = value;
    log("action");
  };

  onMount(async () => {
    window.addEventListener("error", (e) => log(`window-error:${(e.error as Error).message}`));
    count = 1;
    await tick();
    log(`tick:${document.querySelector("#count")?.textContent ?? ""}`);
  });
</script>

<svelte:window bind:innerWidth onkeydown={(e) => log(`window:${e.key}`)} />
<svelte:document onclick={() => log("document:click")} />
<svelte:body onclick={() => log("body:click")} />
<svelte:head><title>svelte head title</title></svelte:head>

<main id="page">
  <section id="toggles">
    <input id="s-cb" type="checkbox" bind:checked={agreed} />
    <label id="s-cb-label" for="s-cb">agree</label>
    <input id="s-mixed" type="checkbox" bind:indeterminate={mixed} />
    <input id="s-apple" type="checkbox" value="apple" bind:group={fruits} />
    <input id="s-pear" type="checkbox" value="pear" bind:group={fruits} />
    <label id="s-l-label"><input id="s-l" type="radio" name="s-size" value="l" bind:group={size} />large</label>
    <input id="s-m" type="radio" name="s-size" value="m" bind:group={size} />
    <p id="s-out">{JSON.stringify({ agreed, mixed, fruits, size })}</p>
  </section>
  <p id="label">{label}</p>
  <button id="inc" onclick={() => count++}>inc</button>
  <p id="count">{count} {doubled}</p>

  {#snippet row(text: string)}<span id="snippet">{text}</span>{/snippet}
  <Child bind:value={childValue} {row} />
  <p id="bound">{childValue}</p>

  {#await promise}<span id="await">pending</span>{:then v}<span id="await">{v}</span>{:catch e}<span id="await"
      >{(e as Error).message}</span
    >{/await}
  <button id="reject" onclick={() => (promise = Promise.reject(new Error("nope")))}>reject</button>

  {#key keyVal}<span id="keyed">{keyVal}</span>{/key}
  <button id="key-bump" onclick={() => keyVal++}>key</button>

  <div id="attached" {@attach tint}>attached</div>
  <div id="action" use:mark={"marked"}>action</div>

  <button id="tr-toggle" onclick={() => (show = !show)}>toggle</button>
  {#if show}<p id="tr" in:fly={{ y: 10, duration: 200 }} out:fade={{ duration: 200 }}>flying</p>{/if}
  <button id="tween-go" onclick={() => ((tween.target = 100), (spring.target = 100))}>go</button>
  <p id="tween">{Math.round(tween.current)} {Math.round(spring.current)}</p>

  <input id="b-text" bind:value={text} />
  <select id="b-select" bind:value={picked}>
    <option value="a">A</option>
    <option value="b">B</option>
    <option value="c">C</option>
  </select>
  <select id="b-many" multiple bind:value={many}>
    <option value="x">X</option>
    <option value="y">Y</option>
  </select>
  <div id="measured" bind:this={el} bind:clientWidth={width} style="width: 123px">measured</div>
  <p id="b-out">{JSON.stringify({ text, picked, many, el: el?.id, width, innerWidth })}</p>

  <button id="flip-go" onclick={() => (list = [...list].reverse())}>flip</button>
  <ul id="flip">
    {#each list as n (n)}<li class="flip-item" animate:flip={{ duration: 500 }}>{n}</li>{/each}
  </ul>

  <svelte:boundary onerror={(e) => log(`boundary:${(e as Error).message}`)}>
    <button
      id="click-boom"
      onclick={() => {
        throw new Error("click-boom");
      }}>click boom</button
    >
  </svelte:boundary>

  <svelte:element this={tag} id="dynamic">dynamic</svelte:element>
  <button id="tag" onclick={() => (tag = "article")}>tag</button>

  <svelte:boundary onerror={(e) => log(`boundary:${(e as Error).message}`)}>
    <Boom fail={boomFail} />
    {#snippet failed(error, reset)}
      <span id="failed">{(error as Error).message}</span>
      <button id="reset" onclick={() => ((boomFail = false), reset())}>reset</button>
    {/snippet}
  </svelte:boundary>
  <button id="boom-go" onclick={() => (boomFail = true)}>boom</button>

  <p id="store">{$store} {$storeDouble}</p>
  <button id="store-inc" onclick={() => store.update((s) => s + 1)}>store</button>

  <LegacyParent />

  <p id="reactivity">{map.get("a") ?? "none"} {set.size} {wide.current}</p>
  <button id="react" onclick={() => (map.set("a", 1), set.add(1))}>react</button>

  <p
    id="classy"
    class={["base", { on: count > 1 }]}
    class:active={count > 1}
    style:color={count > 1 ? "rgb(0, 0, 255)" : "rgb(255, 0, 0)"}
  >
    classy
  </p>
</main>
