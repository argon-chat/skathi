import { cases, IMPORTS } from "../../cli/test/sk-ease-table";
import { shaderTable } from "./_shader-table";

await shaderTable({ label: "shader-ease", imports: IMPORTS, cases: cases() });
