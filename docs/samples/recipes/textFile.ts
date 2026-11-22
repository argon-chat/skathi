const TYPES = [{ description: "Text", accept: { "text/plain": [".txt", ".md"] } }];

let current: FileSystemFileHandle | undefined;

export async function openText(): Promise<string | null> {
  let handle: FileSystemFileHandle | undefined;
  try {
    [handle] = await showOpenFilePicker({ types: TYPES });
  } catch {
    return null;
  }
  if (!handle) return null;
  current = handle;
  return (await handle.getFile()).text();
}

export async function saveText(text: string): Promise<boolean> {
  try {
    current ??= await showSaveFilePicker({ suggestedName: "Untitled.txt", types: TYPES });
  } catch {
    return false;
  }
  const writable = await current.createWritable();
  await writable.write(text);
  await writable.close();
  return true;
}
