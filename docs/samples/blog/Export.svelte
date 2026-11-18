<script lang="ts">
  let formats = $state(["flac"]);
  let quality = $state("high");
</script>

<fieldset>
  <legend>Export as</legend>
  <!-- #region checks -->
  <label><input type="checkbox" value="flac" bind:group={formats} /> FLAC</label>
  <label><input type="checkbox" value="opus" bind:group={formats} /> Opus</label>
  <!-- #endregion checks -->
</fieldset>
<fieldset>
  <legend>Quality</legend>
  <!-- #region radios -->
  <label><input type="radio" value="high" bind:group={quality} /> High</label>
  <label><input type="radio" value="small" bind:group={quality} /> Small</label>
  <!-- #endregion radios -->
</fieldset>
<p>{formats.join(", ") || "Nothing"}, {quality}</p>

<style>
  input {
    accent-color: #c4569a;
  }
</style>
