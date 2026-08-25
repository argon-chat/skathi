<script setup>
// The multi-page shell: RouterLink tabs + RouterView. RouterLink renders an
// <a> (a generic host element here) and stamps `router-link-active` on the
// active one — styled below through the ordinary class matcher.
</script>

<template>
  <div class="shell">
    <div class="nav">
      <RouterLink class="tab" to="/">Forms</RouterLink>
      <RouterLink class="tab" to="/list">List</RouterLink>
      <RouterLink class="tab" to="/motion">Motion</RouterLink>
      <RouterLink class="tab" to="/styles">Styles</RouterLink>
      <RouterLink class="tab" to="/lab">Lab</RouterLink>
    </div>
    <RouterView />
  </div>
</template>

<style scoped>
.shell {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 14px;
}
.nav {
  display: flex;
  flex-direction: row;
  gap: 6px;
}
.tab {
  padding: 6px;
  border-radius: 6px;
  font-size: 12px;
  color: #ffffffaa;
  background: #ffffff0d;
  cursor: pointer;
  transition: background-color 150ms ease, color 150ms ease;
}
.tab:hover {
  background: #ffffff22;
  color: #ffffff;
}
.tab.router-link-active {
  background: #4f8cff59;
  color: #ffffff;
}
</style>
