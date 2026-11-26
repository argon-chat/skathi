import { cases, cost, DECLARATIONS, IMPORTS, parts } from "../../cli/test/sk-noise-table";
import { shaderTable } from "./_shader-table";

await shaderTable({
  label: "shader-noise-2",
  imports: IMPORTS,
  declarations: DECLARATIONS,
  cases: parts(cases(), cost)[1]!,
});
