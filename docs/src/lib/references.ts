import { cssReference } from "./css";
import type { Reference } from "./reference";
import { webReference } from "./web";
import { windowReference } from "./window";

/** The collection pages whose body is a searchable reference catalog, by page id. */
export const references: Readonly<Record<string, Reference>> = {
  [cssReference.id]: cssReference,
  [webReference.id]: webReference,
  [windowReference.id]: windowReference,
};
