// Registration files carry the ES source and UTF-16 import ranges as JSON arguments.
export function registration(script: string): [string, string, unknown[], string] {
  const start = script.lastIndexOf("\n__SK__.defineModule(");
  return JSON.parse("[" + script.slice(start + "\n__SK__.defineModule(".length, script.lastIndexOf(");")) + "]");
}
