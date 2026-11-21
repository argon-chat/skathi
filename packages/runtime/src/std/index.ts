// The `skathi:<name>` modules by name, for the dev loader; a production bundle imports their files.

import * as app from "./app";
import * as binary from "./binary";
import * as collections from "./collections";
import * as encoding from "./encoding";
import * as ffi from "./ffi";
import * as gpu from "./gpu";
import * as network from "./network";
import * as numeric from "./numeric";
import * as os from "./os";
import * as runtime from "./runtime";
import * as text from "./text";
import * as time from "./time";
import * as toml from "./toml";
import * as units from "./units";

export const stdModules: Readonly<Record<string, Record<string, unknown>>> = {
  app,
  binary,
  collections,
  encoding,
  ffi,
  gpu,
  network,
  numeric,
  os,
  runtime,
  text,
  time,
  toml,
  units,
};
