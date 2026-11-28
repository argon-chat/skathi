import { checkCompat } from "./check-compat";
import { compat, overrides } from "../src/data/css/bcd";
import { additionalProperties, entries } from "../src/data/css/entries";

checkCompat("css", entries, overrides, compat, (entry) => additionalProperties.has(entry.name));
