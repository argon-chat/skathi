import { cases, IMPORTS } from "../../cli/test/sk-blend-table";
import { shaderTable } from "./_shader-table";

await shaderTable({ label: "shader-blend", imports: IMPORTS, cases: cases() });
