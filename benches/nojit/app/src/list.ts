import { ref } from "vue";

export const ROWS = 100_000;
export const ROW_HEIGHT = 24;
export const VISIBLE = 40;
/** The list's scroll position in pixels: the rows it renders follow it. */
export const offset = ref(0);
