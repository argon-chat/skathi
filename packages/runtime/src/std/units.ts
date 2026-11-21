// `skathi:units`: sizes of data, frequencies and angles. The host installs the classes.

import type { TimeSpan } from "./time";

export type DataUnit = "B" | "kB" | "MB" | "GB" | "TB" | "PB" | "KiB" | "MiB" | "GiB" | "TiB" | "PiB";

/** A number of bytes. `+size` and `size < other` use that number. */
export interface DataSize {
  readonly bytes: number;
  /** The size in `unit`: `kB` is 1000 bytes, `KiB` is 1024. */
  to(unit: DataUnit): number;
  /**
   * The size in the largest unit that leaves at least 1, with one decimal: `"1.5 GB"`. `binary`
   * counts in 1024s and writes `GiB`. `locale` is the app's by default.
   */
  format(options?: { binary?: boolean; locale?: string | string[] }): string;
  toString(): string;
  valueOf(): number;
}

export interface DataSizeStatics {
  readonly prototype: DataSize;
  bytes(count: number): DataSize;
  /** `"512"`, `"10 MB"`, `"1.5GiB"`; the unit in either case. Throws a `SyntaxError`. */
  parse(text: string): DataSize;
}

export interface Frequency {
  readonly hz: number;
  /** The time one cycle takes. */
  readonly period: TimeSpan;
  /** `"60 Hz"`, `"44.1 kHz"`. */
  toString(): string;
  valueOf(): number;
}

export interface FrequencyStatics {
  readonly prototype: Frequency;
  hz(value: number): Frequency;
  fromPeriod(period: TimeSpan | number): Frequency;
}

export interface Angle {
  readonly degrees: number;
  readonly radians: number;
  /** The same direction from 0° up to, not including, 360°. */
  normalize(): Angle;
  /** `"90°"`. */
  toString(): string;
}

export interface AngleStatics {
  readonly prototype: Angle;
  deg(value: number): Angle;
  rad(value: number): Angle;
}

declare const __sk_std: {
  units: { DataSize: DataSizeStatics; Frequency: FrequencyStatics; Angle: AngleStatics };
};

export const DataSize: DataSizeStatics = __sk_std.units.DataSize;
export const Frequency: FrequencyStatics = __sk_std.units.Frequency;
export const Angle: AngleStatics = __sk_std.units.Angle;
