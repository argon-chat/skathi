// The `skathi:*` modules: what the host itself gives every app, imported by name.

declare module "skathi:app" {
  export * from "@skathi-native/runtime/std/app";
}

declare module "skathi:os" {
  export * from "@skathi-native/runtime/std/os";
}

declare module "skathi:runtime" {
  export * from "@skathi-native/runtime/std/runtime";
}
