import type { LanguageRegistration } from "shiki";
import grammar from "../../vendor/slang/slang.tmLanguage.json";

export default {
  ...grammar,
  name: "slang",
  displayName: "Slang",
} satisfies LanguageRegistration;
