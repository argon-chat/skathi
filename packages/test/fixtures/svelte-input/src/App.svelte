<script lang="ts">
  let name = $state("");
  let clicks = $state(0);
</script>

<input id="name" bind:value={name} placeholder="name" />
<p id="echo">hello {name}</p>
<button id="reset" onclick={() => (name = "reset")}>reset</button>
<a id="link" href="#top" onclick={() => clicks++}>link {clicks}</a>
