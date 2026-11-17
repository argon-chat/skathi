<script setup vapor lang="ts">
import { ref, useCssModule } from "vue";

const body = ref("<b>hi</b>");
const name = ref("");
const items = ref([1, 2]);
const styles = useCssModule();
</script>

<template>
  <div v-html="body"></div>
  <p :innerHTML="body"></p>
  <input type="checkbox" v-model="items" />
  <input type="radio" value="a" />
  <input v-model.lazy="name" />
  <TransitionGroup name="list" tag="ul">
    <li v-for="i in items" :key="i" :class="styles.item">{{ i }}</li>
  </TransitionGroup>
  <TransitionGroup name="fade" tag="ul">
    <li v-for="i in items" :key="i">{{ i }}</li>
  </TransitionGroup>
  <Transition name="bounce"><p v-if="name">bounce</p></Transition>
</template>

<style module>
.item {
  color: red;
}
</style>

<style lang="scss" scoped>
.list-move {
  transition: transform 0.3s;
}
.bounce-leave-active {
  animation: pop 0.3s reverse;
}
</style>
