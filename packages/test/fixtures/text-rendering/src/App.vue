<template>
  <div class="root">
    <div class="pane dark">
      <p class="line on-dark">Settings · Display · Notifications · 14 new</p>
      <p class="line flat-dark">HELM INT 14 TIME LINE</p>
    </div>
    <div class="pane light">
      <p class="line on-light">Settings · Display · Notifications · 14 new</p>
      <p class="line flat-light">HELM INT 14 TIME LINE</p>
      <p class="line subpixel">Settings · Display · Notifications · 14 new</p>
      <p class="line emoji">🙂🎉👍🌍</p>
    </div>
  </div>
</template>

<style>
.root {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  font-family: "Segoe UI", sans-serif;
  font-size: 14px;
}
.pane {
  padding: 12px 16px;
}
.dark {
  background: #1c1c1e;
  color: #ececec;
}
.light {
  background: #ffffff;
  color: #1c1c1e;
}
.line {
  margin: 0 0 8px;
  align-self: flex-start;
}
.subpixel {
  font-smoothing: subpixel-antialiased;
}
.emoji {
  font-size: 20px;
}
</style>
