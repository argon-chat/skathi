import { format as msix } from "./msix";
import type { FormatName, PackageFormat } from "./types";
import { format as dmg } from "./dmg";
import { format as appimage } from "./appimage";

export type FormatRegistry = { readonly [N in FormatName]?: PackageFormat<N> };

export const FORMATS: FormatRegistry = {
  msix,
  dmg,
  appimage,
};
