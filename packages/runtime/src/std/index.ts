// The `skathi:<name>` modules by name, for the dev loader; a production bundle imports their files.

import * as app from "./app";
import * as os from "./os";
import * as runtime from "./runtime";

export const stdModules: Readonly<Record<string, Record<string, unknown>>> = { app, os, runtime };
