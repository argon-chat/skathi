export interface Item {
  id: number;
  label: string;
}

export const list = $state<{ items: Item[]; next: number }>({
  items: [
    { id: 1, label: "one" },
    { id: 2, label: "two" },
  ],
  next: 3,
});

export function add(label: string): void {
  list.items.push({ id: list.next++, label });
}

export function rotate(): void {
  const first = list.items.shift();
  if (first) list.items.push(first);
}
