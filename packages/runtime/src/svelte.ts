// `@skathi-native/runtime/svelte`: mounts a Svelte 5 app on the window's document root.

import {
  type Component,
  type ComponentType,
  mount as svelteMount,
  type MountOptions as SvelteMountOptions,
  type SvelteComponent,
  unmount as svelteUnmount,
} from "svelte";
import { mountDocument } from "@skathi-native/runtime";

/** Svelte's mount options; `target` defaults to the document root. */
export type MountOptions<Props extends object> = Omit<SvelteMountOptions<Props>, "target"> & {
  target?: SvelteMountOptions<Props>["target"];
};

const mounted = new Set<object>();

/** Mounts `component` (on the document root unless `target` says otherwise) and returns its exports. */
export function mount<Props extends object, Exports extends object>(
  component: ComponentType<SvelteComponent<Props>> | Component<Props, Exports>,
  options?: MountOptions<Props>,
): Exports {
  let exports: Exports | undefined;
  mountDocument((root) => {
    const target = options?.target ?? root;
    // svelte-dom-shim: Svelte's DOM mount() renders into the mirror root through the emulated DOM; the custom renderer's mount replaces it (sveltejs/svelte#18042)
    exports = svelteMount(component, { ...options, target } as SvelteMountOptions<Props>) as Exports;
    const app = exports;
    mounted.add(app);
    return () => {
      if (mounted.delete(app)) void svelteUnmount(app);
    };
  });
  return exports!;
}

/** Unmounts what `mount` returned, playing outros when `outro` is set. */
export function unmount(exports: object, options?: { outro?: boolean }): Promise<void> {
  mounted.delete(exports);
  return svelteUnmount(exports as Record<string, unknown>, options);
}
