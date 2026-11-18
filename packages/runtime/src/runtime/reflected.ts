// Reflected IDL attributes (`el.hidden`, `input.placeholder`, `el.tabIndex`): tags (`null`: every HTML element),
// attribute and IDL type. Drives `attributeOf` and `defineReflected`.
type ReflectKind = "bool" | "string" | "long";
export interface Reflected {
  attr: string;
  kind: ReflectKind;
  tags: ReadonlySet<string> | null;
}
export const REFLECTED: Record<string, Reflected> = Object.create(null);
/** `names`: space-separated properties, `prop=attr` where the attribute is not the lowercased name. */
function reflect(kind: ReflectKind, names: string, tags: string | null): void {
  const set = tags ? new Set(tags.split(" ")) : null;
  for (const n of names.split(" ")) {
    const [prop = n, attr = prop.toLowerCase()] = n.split("=");
    REFLECTED[prop] = { attr, kind, tags: set };
  }
}
reflect("bool", "hidden inert autofocus", null);
reflect("bool", "disabled", "button fieldset input link optgroup option select textarea");
reflect("bool", "required", "input select textarea");
reflect("bool", "multiple", "input select");
reflect("bool", "readOnly", "input textarea");
reflect("bool", "open", "details dialog");
reflect("bool", "noValidate", "form");
reflect("bool", "formNoValidate", "button input");
reflect("bool", "reversed", "ol");
reflect("bool", "allowFullscreen", "iframe");
reflect("bool", "isMap", "img");
reflect("bool", "autoplay controls loop", "audio video"); // `loop`'s getter is the media state's (`defineMediaGet`)
reflect("bool", "playsInline", "video");
reflect("bool", "webkitdirectory", "input");
reflect("bool", "alpha hdr", "input");
reflect("bool", "defaultSelected=selected", "option");
reflect("string", "title lang accessKey", null);
reflect("string", "placeholder", "input textarea");
reflect("string", "name", "button fieldset form iframe input map meta object output select slot textarea");
reflect("string", "alt", "area img input");
reflect("string", "allow srcdoc", "iframe");
reflect("string", "referrerPolicy loading", "iframe img");
reflect("string", "accept min max step pattern", "input");
reflect("string", "colorSpace=colorspace", "input"); // its getter is limited to known values (reflect.ts)
reflect("string", "href download rel target", "a area");
reflect("string", "htmlFor=for", "label output");
reflect("long", "tabIndex", null);
reflect("long", "maxLength minLength", "input textarea");
