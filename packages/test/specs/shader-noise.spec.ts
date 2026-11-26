import { cases, cost, DECLARATIONS, IMPORTS, parts } from "../../cli/test/sk-noise-table";
import { shaderTable } from "./_shader-table";

// The first half of the table; shader-noise-2.spec.ts runs the second, so no window waits on fxc for a minute.
await shaderTable({
  label: "shader-noise",
  imports: IMPORTS,
  declarations: DECLARATIONS,
  cases: parts(cases(), cost)[0]!,
});
