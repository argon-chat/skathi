import { cases, cost, DECLARATIONS, IMPORTS, parts } from "../../cli/test/sk-noise-table";
import { shaderTable } from "./_shader-table";

// Combining many noise functions in one fragment program can hang a virtual Metal device.
await shaderTable({
  label: "shader-noise",
  imports: IMPORTS,
  declarations: DECLARATIONS,
  cases: parts(cases(), cost)[0]!,
  casesPerEffect: 3,
});
