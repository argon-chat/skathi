import { cases, IMPORTS } from "../../cli/test/sk-tone-table";
import { shaderTable } from "./_shader-table";

await shaderTable({ label: "shader-tone", imports: IMPORTS, cases: cases() });
