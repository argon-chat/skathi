// The `skathi:*` modules: what the host itself gives every app, imported by name.

declare module "skathi:app" {
  export * from "@skathi-native/runtime/std/app";
}

declare module "skathi:network" {
  export * from "@skathi-native/runtime/std/network";
}

declare module "skathi:numeric" {
  export * from "@skathi-native/runtime/std/numeric";
}

declare module "skathi:os" {
  export * from "@skathi-native/runtime/std/os";
}

declare module "skathi:runtime" {
  export * from "@skathi-native/runtime/std/runtime";
}

declare module "skathi:toml" {
  export * from "@skathi-native/runtime/std/toml";
}
