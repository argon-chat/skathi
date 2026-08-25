// Imported by BOTH .vue files, and itself imports an extensionless directory
// specifier — the case the loader's candidate-extension resolver has to agree
// with the builder about.
import { punctuate } from "./lib";

export function greet(who) {
  return punctuate("hello " + who);
}
