import { parts } from "../../cli/test/sk-noise-table";
import { cases, cost, DECLARATIONS, IMPORTS } from "../../cli/test/sk-sample-table";
import { shaderTable } from "./_shader-table";

// The first half of the table; shader-sample-2.spec.ts runs the second, so no window waits on fxc for a minute.
await shaderTable({
  label: "shader-sample",
  imports: IMPORTS,
  declarations: DECLARATIONS,
  cases: parts(cases(), cost)[0]!,
});
