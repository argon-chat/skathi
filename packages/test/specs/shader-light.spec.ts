import { cases, DECLARATIONS, IMPORTS } from "../../cli/test/sk-light-table";
import { shaderTable } from "./_shader-table";

await shaderTable({ label: "shader-light", imports: IMPORTS, declarations: DECLARATIONS, cases: cases() });
