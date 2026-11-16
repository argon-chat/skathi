<script lang="ts">
  import { appWindow } from "@skathi-native/runtime/svelte";

  const stored = localStorage.getItem("draft") ?? "";
  let text = $state(stored);
  let saved = $state(stored);
  let asking = $state(false);

  function save() {
    localStorage.setItem("draft", text);
    saved = text;
  }

  // #region close
  function onCloseRequested(e: Event) {
    if (text === saved) return;
    e.preventDefault();
    if (appWindow.isMinimized()) appWindow.restore();
    asking = true;
  }

  function answer(choice: "save" | "discard" | "cancel") {
    asking = false;
    if (choice === "cancel") return;
    if (choice === "save") save();
    appWindow.close();
  }
  // #endregion close

  $effect(() => {
    appWindow.addEventListener("close-requested", onCloseRequested);
    return () => appWindow.removeEventListener("close-requested", onCloseRequested);
  });
</script>

<div class="editor">
  <textarea bind:value={text} class="text"></textarea>
  {#if asking}
    <div class="scrim">
      <div class="sheet">
        <p>Save your changes before closing?</p>
        <div class="actions">
          <button class="btn primary" onclick={() => answer("save")}>Save</button>
          <button class="btn" onclick={() => answer("discard")}>Discard</button>
          <button class="btn" onclick={() => answer("cancel")}>Cancel</button>
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .editor {
    position: relative;
    height: 100%;
    padding: 12px;
  }
  .text {
    width: 100%;
    height: 100%;
  }
  .scrim {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: #00000066;
  }
  .sheet {
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 20px;
    border-radius: 8px;
    background: #2b2b2b;
    color: #f3f3f3;
  }
  .actions {
    display: flex;
    gap: 8px;
  }
  .btn {
    padding: 6px 14px;
    border-radius: 6px;
    background: #ffffff14;
    color: #f3f3f3;
  }
  .btn.primary {
    background: #4cc2ff;
    color: #000000;
  }
</style>
