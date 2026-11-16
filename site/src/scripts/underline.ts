// The footer's links (`.fl`): the underline that draws in on hover grows from where the pointer
// came in, and draws back towards where it went out. The place is set as --ux on the link; with
// no pointer (the keyboard's focus, or no script) it draws from the left.

export function bindUnderlines(root: Element | null = document.querySelector(".foot")): void {
  if (!root) return;
  const from = (e: Event) => {
    const a = (e.target as Element).closest<HTMLElement>(".fl");
    if (!a) return;
    const r = a.getBoundingClientRect();
    const x = r.width ? ((e as PointerEvent).clientX - r.left) / r.width : 0;
    a.style.setProperty("--ux", `${Math.round(Math.min(1, Math.max(0, x)) * 100)}%`);
  };
  root.addEventListener("pointerover", from);
  root.addEventListener("pointerout", from);
}
