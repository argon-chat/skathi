import { parts } from "../../cli/test/sk-noise-table";
import { cases, cost, DECLARATIONS, IMPORTS } from "../../cli/test/sk-sample-table";
import { shaderTable } from "./_shader-table";

await shaderTable({
  label: "shader-sample-2",
  imports: IMPORTS,
  declarations: DECLARATIONS,
  cases: parts(cases(), cost)[1]!,
});
