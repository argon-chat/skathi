<script lang="ts">
  import { SkWindow } from "@skathi-native/runtime/svelte";

  let popped = $state(false);
  let mainFile = $state("idle");
  let childFile = $state("idle");
  let result = $state("idle");
  let original: File;
  function begin(e: DragEvent) {
    const store = e.dataTransfer!;
    store.effectAllowed = "copyMove";
    store.setData("application/x-skathi-test", "private");
    store.setData("text/plain", "public");
    original = new File(["picture"], "drawing.png", { type: "image/png" });
    store.items.add(original);
    store.setDragImage(e.currentTarget as Element, 3, 4);
  }
  function over(e: DragEvent) {
    if (e.dataTransfer?.types.includes("application/x-skathi-test")) {
      e.preventDefault();
      const dt = e.dataTransfer;
      dt.dropEffect = "move";
      result = [dt.files.length, dt.getData("application/x-skathi-test"), dt.effectAllowed].join("/");
    } else if (e.dataTransfer?.types.includes("Files")) {
      e.preventDefault();
      e.dataTransfer.dropEffect = "copy";
    }
  }
  function drop(e: DragEvent, child: boolean) {
    e.preventDefault();
    const dt = e.dataTransfer!;
    const name = dt.types.includes("application/x-skathi-test")
      ? [
          dt.getData("application/x-skathi-test"),
          dt.getData("text/plain"),
          dt.files[0] === original,
          dt.files[0]?.name,
        ].join("/")
      : (dt.files[0]?.name ?? "empty");
    if (child) childFile = name;
    else mainFile = name;
  }
</script>

<button id="open" onclick={() => (popped = true)}>Open drop window</button>
<button
  id="drag-source"
  draggable="true"
  ondragstart={begin}
  ondragend={(e) => (mainFile = e.dataTransfer?.dropEffect ?? "missing")}>Drag drawing</button
>
<output id="drag-preview">{result}</output>
<div
  role="group"
  aria-label="Main window files"
  id="svelte-main"
  style="height: 45px;"
  ondragover={over}
  ondrop={(e) => drop(e, false)}
>
  {mainFile}
</div>
{#if popped}
  <SkWindow title="File drop" width={300} height={160}>
    <div
      role="group"
      aria-label="Child window files"
      id="svelte-child"
      style="width: 180px; height: 65px;"
      ondragover={over}
      ondrop={(e) => drop(e, true)}
    >
      {childFile}
    </div>
  </SkWindow>
{/if}
