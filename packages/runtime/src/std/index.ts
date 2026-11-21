// The `skathi:<name>` modules by name, for the dev loader; a production bundle imports their files.

import * as app from "./app";
import * as binary from "./binary";
import * as encoding from "./encoding";
import * as network from "./network";
import * as numeric from "./numeric";
import * as os from "./os";
import * as runtime from "./runtime";
import * as toml from "./toml";

export const stdModules: Readonly<Record<string, Record<string, unknown>>> = {
  app,
  binary,
  encoding,
  network,
  numeric,
  os,
  runtime,
  toml,
};
