// The comparison counts up once, the first time it's 40 % on screen: each figure from zero
// to its value, each bar from nothing to its length. A figure without `data-to` (a range of
// processes, nothing to download) keeps its text and only its bar grows. The final values
// are in the HTML, so without script (or with reduced motion) they simply show.
import { formatFigure } from "../data/format";
import type { NumericMetric } from "../data/numbers";

const DURATION = 1100;
const ease = (t: number) => 1 - Math.pow(1 - t, 3);

type Item = { value: HTMLElement; bar: HTMLElement | null; metric: NumericMetric; to: number | null; scale: number };

export function bindCountUp(chart: HTMLElement): void {
  if (matchMedia("(prefers-reduced-motion: reduce)").matches) {
    chart.dataset.counted = "true";
    return;
  }
  const items: Item[] = [...chart.querySelectorAll<HTMLElement>("[data-figure]")].map((el) => ({
    value: el.querySelector<HTMLElement>("[data-value]")!,
    bar: el.querySelector<HTMLElement>("[data-bar]"),
    metric: el.dataset.metric as NumericMetric,
    to: el.dataset.to === undefined ? null : Number(el.dataset.to),
    scale: Number(el.dataset.scale),
  }));

  const draw = (k: number) => {
    for (const it of items) {
      if (it.to !== null) it.value.textContent = formatFigure(it.metric, it.to * k);
      if (it.bar) it.bar.style.transform = `scaleX(${(it.scale * k).toFixed(4)})`;
    }
  };

  // Not seen yet: wait at zero.
  const rect = chart.getBoundingClientRect();
  if (rect.top < innerHeight * 0.6 && rect.bottom > 0) {
    chart.dataset.counted = "true";
    return;
  }
  draw(0);

  const io = new IntersectionObserver(
    (entries) => {
      if (!entries.some((e) => e.isIntersecting)) return;
      io.disconnect();
      const start = performance.now();
      const step = (t: number) => {
        const k = Math.min(1, (t - start) / DURATION);
        draw(ease(k));
        if (k < 1) requestAnimationFrame(step);
        else chart.dataset.counted = "true";
      };
      requestAnimationFrame(step);
    },
    { threshold: 0.4 },
  );
  io.observe(chart);
}
