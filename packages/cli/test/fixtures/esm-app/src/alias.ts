// Reaches util.ts only through the tsconfig alias: the graph edge and the one
// module instance both depend on the dev builder resolving it.
import { state } from "@/util";
import config from "@/data/config.json";

export const viaAlias = state;
export const aliasedJson = config;
