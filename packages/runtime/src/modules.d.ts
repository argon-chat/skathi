// Non-JS files an app imports for their side effect; the builder bundles them.

declare module "*.css" {}

/** An audio worklet module, built into one classic script: the URL to give `audioWorklet.addModule()`. */
declare module "*?worklet" {
  const url: string;
  export default url;
}
