import { checkCompat } from "./check-compat";
import { compat, overrides } from "../src/data/web/bcd";
import { entries } from "../src/data/web/entries";

checkCompat("web", entries, overrides, compat);
