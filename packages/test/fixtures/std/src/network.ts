import { network } from "skathi:network";

interface FakeNetwork {
  online?: boolean;
  type?: ConnectionType;
  metered?: boolean | null;
}

// Defined by the host only with SK_NETWORK=fake.
declare const __SK_TEST_NETWORK__: (state: FakeNetwork) => void;

export const connection = () => ({
  type: navigator.connection.type,
  same: navigator.connection.type === network.type,
  instance: navigator.connection instanceof NetworkInformation,
  target: navigator.connection instanceof EventTarget,
  measured: ["effectiveType", "downlink", "downlinkMax", "rtt", "saveData"].filter((k) => k in navigator.connection),
});

const heard = (target: EventTarget, type: string) =>
  new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`no ${type} event`)), 5000);
    target.addEventListener(type, () => (clearTimeout(timer), resolve()), { once: true });
  });

export async function connectionChange() {
  const c = navigator.connection;
  const log: string[] = [];
  for (const type of ["change", "online", "offline"]) network.addEventListener(type, () => log.push(`network:${type}`));
  c.addEventListener("change", () => log.push(`connection:${c.type}`));
  c.onchange = () => log.push("onchange");
  const steps: [string, boolean][] = [];
  const step = async (state: FakeNetwork) => {
    const done = heard(network, "change");
    __SK_TEST_NETWORK__(state);
    await done;
    // navigator.connection hears it after network does, in the same task.
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
    steps.push([c.type, c.type === network.type && navigator.onLine === network.online]);
  };
  await step({ type: "wifi", online: true });
  await step({ metered: true });
  await step({ type: "none", online: false });
  return { log, steps, metered: network.metered, onLine: navigator.onLine };
}
