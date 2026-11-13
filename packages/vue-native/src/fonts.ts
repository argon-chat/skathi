// Native CSS Font Loading lives in vn-tree/font_api.rs. Only the mirror
// document attachment belongs to Vue. These declarations emit no runtime code.
export interface NativeFontFace {
  family: string;
  style: string;
  weight: string;
  stretch: string;
  unicodeRange: string;
  display: string;
  readonly status: "unloaded" | "loading" | "loaded" | "error";
  readonly loaded: Promise<NativeFontFace>;
  load(): Promise<NativeFontFace>;
}
export interface NativeFontFaceSet extends Iterable<NativeFontFace> {
  readonly size: number;
  readonly status: "loading" | "loaded";
  readonly ready: Promise<NativeFontFaceSet>;
  add(face: NativeFontFace): this;
  delete(face: NativeFontFace): boolean;
  clear(): void;
  has(face: NativeFontFace): boolean;
  check(font: string, text?: string): boolean;
  load(font: string, text?: string): Promise<NativeFontFace[]>;
}
interface FontHost {
  __vn_fonts_attach?: (document: object) => NativeFontFaceSet;
}
export function installFonts(
  global: Record<string, unknown>,
  document: object | null | undefined,
): NativeFontFaceSet | null {
  const attach = (global as FontHost).__vn_fonts_attach;
  return document && typeof attach === "function" ? attach(document) : null;
}
