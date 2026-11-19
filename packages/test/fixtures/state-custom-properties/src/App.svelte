<div class="row">
  <div id="self" class="box self"></div>
  <div id="parent" class="box">
    <div id="child" class="child"></div>
    <span id="unpainted"></span>
  </div>
  <div id="inline" class="box">
    <svg width="60" height="32">
      <defs>
        <linearGradient id="inline-gradient">
          <stop offset="0" style="stop-color: var(--c)" />
          <stop offset="1" style="stop-color: var(--c)" />
        </linearGradient>
      </defs>
      <rect width="60" height="32" fill="url(#inline-gradient)" />
    </svg>
  </div>
  <div id="sheet" class="box">
    <svg width="60" height="32">
      <defs>
        <linearGradient id="sheet-gradient">
          <stop class="token" offset="0" />
          <stop class="token" offset="1" />
        </linearGradient>
      </defs>
      <rect width="60" height="32" fill="url(#sheet-gradient)" />
    </svg>
  </div>
  <div id="color" class="box">
    <svg width="60" height="32"><rect width="60" height="32" fill="currentColor" /></svg>
  </div>
  <div id="same" class="box same"></div>
  <div id="geometry" class="box">
    <svg width="60" height="32">
      <rect width="60" height="32" fill="red" />
      <rect class="stripe" height="32" fill="blue" />
    </svg>
  </div>
</div>
<div id="motion"><div id="bar"></div></div>
<div class="focus-group"><button id="action"><span id="label">Action</span></button></div>
<button id="other">Other</button>

<style>
  .row {
    display: flex;
    gap: 12px;
    padding: 12px;
  }
  .box,
  .child {
    width: 60px;
    height: 32px;
  }
  .box {
    --c: red;
    --stripe: 12px;
    color: red;
  }
  .box:hover {
    --c: blue;
    --stripe: 48px;
    color: blue;
  }
  .self,
  .child,
  .same:hover {
    background: var(--c);
  }
  .same {
    background-color: red;
  }
  .token {
    stop-color: var(--c);
  }
  .stripe {
    width: var(--stripe);
  }

  #motion {
    width: 120px;
    height: 30px;
    --bar-width: 20px;
    --duration: 450ms;
  }
  #motion:hover {
    --bar-width: 90px;
  }
  #bar {
    height: 18px;
    width: var(--bar-width);
    background-color: blue;
    transition: width var(--duration) linear;
  }
  .focus-group {
    --within: outside;
  }
  .focus-group:focus-within {
    --within: inside;
  }
  #action {
    --focus: off;
    --press: up;
    --both: no;
  }
  #action:focus {
    --focus: on;
  }
  #action:active {
    --press: down;
  }
  #action:hover:focus {
    --both: yes;
  }
</style>
