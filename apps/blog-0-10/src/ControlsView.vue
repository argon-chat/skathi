<template>
  <form class="controls" @submit.prevent>
    <div class="line">
      <button type="button">Button</button>
      <button type="button" disabled>Disabled</button>
      <input type="submit" value="Submit" />
      <input type="reset" value="Reset" />
    </div>
    <div class="line">
      <input type="text" placeholder="Text input" />
      <input type="number" value="42" />
    </div>
    <div class="line">
      <input type="password" value="secret" />
      <input type="search" placeholder="Search" />
    </div>
    <div class="line">
      <label><input type="checkbox" checked /> Checkbox</label>
      <label><input type="radio" name="r" checked /> Radio</label>
      <label><input type="radio" name="r" /> Other</label>
      <input type="range" value="60" />
    </div>
    <div class="line">
      <select>
        <option>Select</option>
        <option>Second</option>
      </select>
      <input type="color" value="#b9a2ff" />
      <input type="text" value="Disabled" disabled />
    </div>
    <textarea placeholder="Textarea"></textarea>
  </form>
</template>

<style scoped>
.controls {
  display: flex;
  flex-direction: column;
  gap: 12px;
  width: 100%;
}
.line {
  display: flex;
  align-items: center;
  gap: 10px;
}
</style>
