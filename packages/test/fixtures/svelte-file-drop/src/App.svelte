<script lang="ts">
  import { SkWindow } from "@skathi-native/runtime/svelte";

  let popped = $state(false);
  let mainFile = $state("idle");
  let childFile = $state("idle");
  function over(e: DragEvent) {
    if (e.dataTransfer?.types.includes("Files")) {
      e.preventDefault();
      e.dataTransfer.dropEffect = "copy";
    }
  }
  function drop(e: DragEvent, child: boolean) {
    e.preventDefault();
    const name = e.dataTransfer?.files[0]?.name ?? "empty";
    if (child) childFile = name;
    else mainFile = name;
  }
</script>

<button id="open" onclick={() => (popped = true)}>Open drop window</button>
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
