import { cases, DECLARATIONS, IMPORTS } from "../../cli/test/sk-sdf-table";
import { shaderTable } from "./_shader-table";

await shaderTable({ label: "shader-sdf", imports: IMPORTS, declarations: DECLARATIONS, cases: cases() });
