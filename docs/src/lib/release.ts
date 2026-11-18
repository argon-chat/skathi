import cli from "../../../packages/cli/package.json";

export const version = cli.version;

function minor(release: string): string {
  return release.split(".").slice(0, 2).join(".");
}

export const currentMinor = minor(version);

/** Whether `since`, "0.4" or "0.4.0", is the minor release going out now: what wears a New badge. */
export function isNew(since: string | undefined): boolean {
  return since !== undefined && minor(since) === currentMinor;
}

export const newLabel = `New in ${currentMinor}`;
