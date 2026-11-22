// Non-JS files an app imports for their side effect; the builder bundles them.

declare module "*.css" {}

/** A TOML document, as `TOML.parse` from `skathi:toml` reads it when the app starts. */
declare module "*.toml" {
  const document: Record<string, unknown>;
  export default document;
}

/** An audio worklet module, built into one classic script: the URL to give `audioWorklet.addModule()`. */
declare module "*?worklet" {
  const url: string;
  export default url;
}
