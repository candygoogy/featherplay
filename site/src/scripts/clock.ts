// The taskbar clock: the visitor's own time, in their own format, redrawn on the minute
// (never more often), and not at all while the page is hidden.

function draw(el: HTMLElement, now: Date) {
  const time = now.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  const date = now.toLocaleDateString(undefined, { day: "2-digit", month: "2-digit", year: "numeric" });
  const t = el.querySelector<HTMLElement>("[data-clock-time]");
  const d = el.querySelector<HTMLElement>("[data-clock-date]");
  if (t) t.textContent = time;
  if (d) d.textContent = date;
  el.setAttribute("datetime", now.toISOString());
}

/** Starts the clock in `el`; the returned function stops it. */
export function startClock(el: HTMLElement): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const tick = () => {
    const now = new Date();
    draw(el, now);
    timer = setTimeout(tick, 60_000 - (now.getSeconds() * 1000 + now.getMilliseconds()) + 5);
  };
  const onVisibility = () => {
    clearTimeout(timer);
    if (document.visibilityState === "visible") tick();
  };
  document.addEventListener("visibilitychange", onVisibility);
  tick();
  return () => {
    clearTimeout(timer);
    document.removeEventListener("visibilitychange", onVisibility);
  };
}
