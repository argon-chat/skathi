import { join } from "node:path";

export default {
  content: [join(__dirname, "src/**/*.{vue,ts}")],

  corePlugins: {
    // Preflight would only spray element and pseudo-element selectors (`*, ::before, ::after`,
    // `html`, `abbr:where(...)`) that vn-css cannot match — pure diagnostic noise.
    preflight: false,

    backgroundOpacity: false,
    textOpacity: false,
    borderOpacity: false,
    divideOpacity: false,
    placeholderOpacity: false,
    ringOpacity: false,
  },

  theme: { extend: {} },
  plugins: [],
};
