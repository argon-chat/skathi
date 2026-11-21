// The `skathi:<name>` modules by name, for the dev loader; a production bundle imports their files.

import * as app from "./app";
import * as network from "./network";
import * as numeric from "./numeric";
import * as os from "./os";
import * as runtime from "./runtime";
import * as toml from "./toml";

export const stdModules: Readonly<Record<string, Record<string, unknown>>> = {
  app,
  network,
  numeric,
  os,
  runtime,
  toml,
};
