import { describe, expect, it } from "vitest";
import { placeCard } from "../../src/demo/tour";

const box = (left: number, top: number, w: number, h: number) => ({ left, top, right: left + w, bottom: top + h });
const overlaps = (p: { x: number; y: number }, w: number, h: number, b: ReturnType<typeof box>) =>
  p.x < b.right && b.left < p.x + w && p.y < b.bottom && b.top < p.y + h;

describe("where the tour card goes", () => {
  const card = { w: 216, h: 240 };

  it("beside the laptop, on the target's side, when the page has room", () => {
    const laptop = box(280, 200, 880, 580);
    const screen = box(330, 210, 780, 488);
    const left = placeCard(card, laptop, screen, { width: 1440 }, box(400, 600, 30, 30), []);
    expect(left).toMatchObject({ beside: true, x: 280 - 216 - 16 });
    const right = placeCard(card, laptop, screen, { width: 1440 }, box(1000, 300, 30, 30), []);
    expect(right).toMatchObject({ beside: true, x: 1160 + 16 });
  });

  it("on the screen when there's no room, clear of the target and the memory panel", () => {
    const laptop = box(40, 200, 1000, 640);
    const screen = box(90, 210, 900, 560);
    const target = box(950, 230, 24, 24); // the minimise button, top right
    const meter = box(700, 450, 290, 300); // the memory flyout, bottom right
    const p = placeCard(card, laptop, screen, { width: 1080 }, target, [meter]);
    expect(p.beside).toBe(false);
    expect(overlaps(p, card.w, card.h, meter)).toBe(false);
    expect(overlaps(p, card.w, card.h, target)).toBe(false);
    expect(p.x).toBeGreaterThanOrEqual(screen.left);
    expect(p.x + card.w).toBeLessThanOrEqual(screen.right);
  });
});
