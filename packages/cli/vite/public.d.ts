import type { Plugin, PluginOption } from "vite";

export interface NativeVitePluginOptions {
  appRoot: string;
  aliases?: Record<string, string>;
  nativeSvelte?: boolean;
}

/** Compose the native renderer plugins with the plugins from an application's Vite config. */
export declare function nativeVitePlugins(
  options: NativeVitePluginOptions,
  userPlugins?: PluginOption[],
): Promise<Plugin[]>;
