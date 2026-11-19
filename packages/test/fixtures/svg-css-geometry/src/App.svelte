<div class="row">
  <svg id="outline" class="box"><rect class="outline" stroke="#818cf8" fill="none" /></svg>
  <svg id="outline-control" class="box">
    <rect x="0.5" y="0.5" width="119" height="43" rx="7.5" stroke="#818cf8" fill="none" />
  </svg>
  <svg id="gradient" class="box">
    <defs
      ><linearGradient id="g3">
        <stop offset="0" style="stop-color: var(--a)" />
        <stop class="last" offset="1" />
      </linearGradient></defs
    >
    <rect width="120" height="44" fill="url(#g3)" />
  </svg>
  <svg id="gradient-control" class="box">
    <defs
      ><linearGradient id="g4">
        <stop offset="0" stop-color="#6366f1" />
        <stop offset="1" stop-color="#c084fc" stop-opacity="0.75" />
      </linearGradient></defs
    >
    <rect width="120" height="44" fill="url(#g4)" />
  </svg>
  <svg class="bad"><rect class="missing-width" height="4" /></svg>
</div>

<style>
  .row {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 8px;
    background: #111;
    --a: #6366f1;
    --b: #c084fc;
  }
  .box {
    width: 120px;
    height: 44px;
  }
  .outline {
    x: 0.5px;
    y: 0.5px;
    width: calc(100% - 1px);
    height: calc(100% - 1px);
    rx: 7.5px;
  }
  .last {
    stop-color: var(--b);
    stop-opacity: 75%;
  }
  .bad {
    width: 4px;
    height: 4px;
  }
</style>
