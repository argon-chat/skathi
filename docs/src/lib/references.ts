import { appReference } from "./app";
import { cssReference } from "./css";
import type { Reference } from "./reference";
import { stdReference } from "./std";
import { webReference } from "./web";
import { windowReference } from "./window";

/** The collection pages whose body is a searchable reference catalog, by page id. */
export const references: Readonly<Record<string, Reference>> = {
  [cssReference.id]: cssReference,
  [webReference.id]: webReference,
  [windowReference.id]: windowReference,
  [appReference.id]: appReference,
  [stdReference.id]: stdReference,
};
