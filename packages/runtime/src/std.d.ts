// The `skathi:*` modules: what the host itself gives every app, imported by name.

declare module "skathi:app" {
  export * from "@skathi-native/runtime/std/app";
}

declare module "skathi:binary" {
  export * from "@skathi-native/runtime/std/binary";
}

declare module "skathi:collections" {
  export * from "@skathi-native/runtime/std/collections";
}

declare module "skathi:encoding" {
  export * from "@skathi-native/runtime/std/encoding";
}

declare module "skathi:ffi" {
  export * from "@skathi-native/runtime/std/ffi";
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

declare module "skathi:time" {
  export * from "@skathi-native/runtime/std/time";
}

declare module "skathi:toml" {
  export * from "@skathi-native/runtime/std/toml";
}

declare module "skathi:units" {
  export * from "@skathi-native/runtime/std/units";
}
