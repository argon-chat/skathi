import { OK, importLine, section } from "./shared";

export const units = section("skathi:units", [
  {
    slug: "data-size",
    name: "DataSize",
    kind: "class",
    summary: "A number of bytes that parses, converts between decimal and binary units, and formats for a locale.",
    syntax: `${importLine("skathi:units", "DataSize")}

type DataUnit = "B" | "kB" | "MB" | "GB" | "TB" | "PB" | "KiB" | "MiB" | "GiB" | "TiB" | "PiB"

DataSize.bytes(count: number): DataSize
DataSize.parse(text: string): DataSize

readonly size.bytes: number
size.to(unit: DataUnit): number
size.format(options?: { binary?: boolean; locale?: string | string[] }): string
size.toString(): string
size.valueOf(): number`,
    values: [
      { value: "B", status: OK, note: "One byte." },
      { value: "kB", status: OK, note: "Decimal: 1000 bytes." },
      { value: "MB", status: OK, note: "Decimal: 1000 kB." },
      { value: "GB", status: OK, note: "Decimal: 1000 MB." },
      { value: "TB", status: OK, note: "Decimal: 1000 GB." },
      { value: "PB", status: OK, note: "Decimal: 1000 TB." },
      { value: "KiB", status: OK, note: "Binary: 1024 bytes." },
      { value: "MiB", status: OK, note: "Binary: 1024 KiB." },
      { value: "GiB", status: OK, note: "Binary: 1024 MiB." },
      { value: "TiB", status: OK, note: "Binary: 1024 GiB." },
      { value: "PiB", status: OK, note: "Binary: 1024 TiB." },
    ],
    notes: [
      "A size is made by `DataSize.bytes()` or `DataSize.parse()`; the type has no public constructor. A size is immutable.",
      "The units are `B`, the decimal `kB`, `MB`, `GB`, `TB`, `PB` (1000 bytes to the step) and the binary `KiB`, `MiB`, `GiB`, `TiB`, `PiB` (1024).",
      '`parse()` reads a number and a unit, with or without a space between them: `"10 MB"`, `"1.5GiB"`. The unit may be in either case. A bare number is a number of bytes: `"512"`.',
      "`to(unit)` gives the size in that unit as a number. Here the unit is written exactly as in the list.",
      '`format()` picks the largest unit that leaves at least 1 and keeps one decimal: `"1.5 GB"`. `binary: true` counts in 1024s and writes `GiB`. A size under one kB or KiB is written in bytes.',
      'The number is written for `locale`, the app\'s by default, by `Intl.NumberFormat`: `"1,5 MB"` in German. For the decimal units `Intl.NumberFormat` writes the unit as well, in its short form. The binary unit names are always written as in the list.',
      'With [`"intl": "invariant"`](/sk-config/#intl) in sk.config.json, the text is the root locale\'s form, whatever locale is asked for.',
      '`toString()` is `format()` without options, so `String(size)` gives `"1.5 GB"` in the app\'s locale.',
      "`valueOf()` gives the bytes, so `+size` and `a < b` work on sizes.",
    ],
    examples: [
      {
        title: "Parse, convert and format",
        lang: "ts",
        code: `import { DataSize } from "skathi:units";

const size = DataSize.parse("1.5 GiB");

size.bytes; // 1610612736
size.to("MiB"); // 1536
DataSize.bytes(2_500).to("kB"); // 2.5

DataSize.bytes(1_500_000_000).format({ locale: "en" }); // "1.5 GB"
size.format({ binary: true, locale: "en" }); // "1.5 GiB"
DataSize.bytes(1_500_000).format({ locale: "de" }); // "1,5 MB"

DataSize.parse("1 MB") > DataSize.parse("999 kB"); // true`,
      },
      {
        title: "Refuse a file over a configured limit",
        lang: "ts",
        code: `import { DataSize } from "skathi:units";

const limit = DataSize.parse("25 MB");

export function check(file: File): string | null {
  const size = DataSize.bytes(file.size);
  if (size <= limit) return null;
  return file.name + " is " + size.format() + ". The limit is " + limit.format() + ".";
}`,
      },
    ],
    edgeCases: [
      '`DataSize.parse()` throws a `SyntaxError` for text that does not start with a number or that names an unknown unit (`"3 parsecs"`). A leading sign is not read.',
      "`DataSize.parse()` throws a `TypeError` when the argument is not a string.",
      "`DataSize.bytes()` throws a `RangeError` when the count is not a finite number.",
      '`to()` throws a `TypeError` for a unit that is not in the list. It does not accept another case: `"mb"` throws.',
    ],
    related: ["frequency", "angle", "time-span"],
    aliases: [
      "bytes",
      "file size",
      "human readable size",
      "kilobyte",
      "megabyte",
      "gigabyte",
      "kibibyte",
      "mebibyte",
      "gibibyte",
      "DataUnit",
      "DataSize.bytes",
      "DataSize.parse",
      "DataSize.to",
      "DataSize.format",
      "DataSize.toString",
    ],
  },
  {
    slug: "frequency",
    name: "Frequency",
    kind: "class",
    summary: "A frequency in hertz, with the time one cycle takes.",
    syntax: `${importLine("skathi:units", "Frequency")}

Frequency.hz(value: number): Frequency
Frequency.fromPeriod(period: TimeSpan | number): Frequency

readonly frequency.hz: number
readonly frequency.period: TimeSpan
frequency.toString(): string
frequency.valueOf(): number`,
    notes: [
      "A frequency is made by `Frequency.hz()` or `Frequency.fromPeriod()`; the type has no public constructor. A frequency is immutable.",
      "`period` is the time one cycle takes, as a [TimeSpan](/std/time-span/). `fromPeriod()` takes a `TimeSpan` or a number of milliseconds.",
      '`toString()` writes the value in Hz, kHz, MHz or GHz with up to three decimals: `"60 Hz"`, `"44.1 kHz"`. The text does not depend on the locale.',
      "`valueOf()` gives the hertz, so `+frequency` and `a < b` work on frequencies.",
    ],
    examples: [
      {
        title: "Frequency and period",
        lang: "ts",
        code: `import { TimeSpan } from "skathi:time";
import { Frequency } from "skathi:units";

Frequency.hz(50).period.totalMilliseconds; // 20
Frequency.fromPeriod(TimeSpan.fromSeconds(2)).hz; // 0.5

String(Frequency.hz(44_100)); // "44.1 kHz"
String(Frequency.fromPeriod(1)); // "1 kHz"`,
      },
      {
        title: "Run a simulation step at a fixed rate",
        lang: "ts",
        code: `import { Timer } from "skathi:time";
import { Frequency } from "skathi:units";

const rate = Frequency.hz(20);

export function simulate(step: (seconds: number) => void): Timer {
  const timer = new Timer(rate.period, { repeat: true });
  timer.addEventListener("tick", () => step(rate.period.totalSeconds));
  timer.start();
  return timer;
}`,
      },
    ],
    edgeCases: [
      "`Frequency.hz()` throws a `RangeError` when the value is negative or not a finite number.",
      "`Frequency.fromPeriod()` throws a `RangeError` when the period is 0 or less, or not a finite number or a `TimeSpan`.",
      "Reading `period` of a frequency of 0 Hz throws a `RangeError`: 0 Hz has no period.",
    ],
    related: ["time-span", "timer", "data-size", "angle"],
    aliases: [
      "hertz",
      "Hz",
      "kHz",
      "refresh rate",
      "sample rate",
      "period",
      "Frequency.hz",
      "Frequency.fromPeriod",
      "Frequency.period",
    ],
  },
  {
    slug: "angle",
    name: "Angle",
    kind: "class",
    summary: "An angle that reads in degrees and radians and normalizes to one turn.",
    syntax: `${importLine("skathi:units", "Angle")}

Angle.deg(value: number): Angle
Angle.rad(value: number): Angle

readonly angle.degrees: number
readonly angle.radians: number
angle.normalize(): Angle
angle.toString(): string`,
    notes: [
      "An angle is made by `Angle.deg()` or `Angle.rad()`; the type has no public constructor. An angle is immutable.",
      "`normalize()` returns a new angle with the same direction, from 0° up to, not including, 360°.",
      '`toString()` writes the degrees with up to three decimals and the degree sign: `"90°"`. The text does not depend on the locale.',
      "An `Angle` has no `valueOf()`. Compare `degrees` or `radians`.",
    ],
    examples: [
      {
        title: "Convert and normalize",
        lang: "ts",
        code: `import { Angle } from "skathi:units";

Angle.rad(Math.PI).degrees; // 180
Angle.deg(180).radians === Math.PI; // true
Angle.deg(-90).normalize().degrees; // 270
String(Angle.deg(450).normalize()); // "90°"`,
      },
      {
        title: "Turn a dial by the pointer position",
        lang: "ts",
        code: `import { Angle } from "skathi:units";

export function dialAngle(dial: HTMLElement, event: PointerEvent): Angle {
  const box = dial.getBoundingClientRect();
  const x = event.clientX - (box.left + box.width / 2);
  const y = event.clientY - (box.top + box.height / 2);
  return Angle.rad(Math.atan2(y, x)).normalize();
}

export function rotate(dial: HTMLElement, angle: Angle): void {
  dial.style.transform = "rotate(" + angle.degrees + "deg)";
}`,
      },
    ],
    edgeCases: ["`Angle.deg()` and `Angle.rad()` throw a `RangeError` when the value is not a finite number."],
    related: ["frequency", "data-size"],
    aliases: [
      "degrees",
      "radians",
      "rotation",
      "deg to rad",
      "Angle.deg",
      "Angle.rad",
      "Angle.normalize",
      "Angle.degrees",
      "Angle.radians",
    ],
  },
]);
