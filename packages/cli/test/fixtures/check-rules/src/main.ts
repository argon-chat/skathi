import * as builtins from "./builtins";
import * as clean from "./clean";
import * as dynamic from "./dynamic";
import * as timers from "./legacy/timers";
import * as typed from "./typing";
import { patch } from "patchy";
import { loose } from "loose-types";
import evil from "evil-eval";
import shorthand from "shorthand-pkg";
import { hidden } from "hidden-types";
import plain from "./plain.js";
import App from "./App.vue";
import { libraries } from "./native";
import { kit } from "native-kit";

console.log(builtins, clean, dynamic, timers, typed, patch, loose, evil, shorthand, hidden, plain, App, libraries, kit);
