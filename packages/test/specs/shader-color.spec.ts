import { cases } from "../../cli/test/sk-color-table";
import { shaderTable } from "./_shader-table";

await shaderTable({ label: "shader-color", imports: ["sk.color"], cases: cases() });
