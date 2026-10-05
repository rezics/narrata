import { expect, test } from "vitest";
import { createLabelPlacement, graphCanvasLabel, projectionGroups } from "../dist/graph/labels.js";

test("canvas labels use resolved titles and abbreviate only ID placeholders", () => {
  const id = "01a1095c2e43745d8de74c7a024de25d";
  expect(graphCanvasLabel("旧驿站", id)).toBe("旧驿站");
  expect(graphCanvasLabel(`〔${id}〕`, id)).toBe("〔01a1…e25d〕");
  expect(graphCanvasLabel(`〔${id.slice(0, -4)}abcd〕`, `${id.slice(0, -4)}abcd`)).toBe("〔01a1…abcd〕");
  const groups = projectionGroups([{ id: "a", x: 120.4998, y: 240 }, { id: "b", x: 120.5002, y: 240 }, { id: "c", x: 123, y: 240 }]);
  expect(groups.get("a")).toEqual({ key: "a", count: 2 });
  expect(groups.get("b")).toEqual(groups.get("a"));
  expect(groups.get("c")?.count).toBe(1);
});

test("labels stay inside the canvas and use another side or disappear when space is occupied", () => {
  const place = createLabelPlacement(390, 480);
  const nearRight = place(345, 430, 5, 90, 17);
  expect(nearRight).toEqual({ left: 244, right: 334, top: 421.5, bottom: 438.5 });
  const adjacent = place(330, 430, 5, 90, 17);
  expect(adjacent).not.toBeNull();
  expect(adjacent!.top).toBeGreaterThan(nearRight!.bottom);
  expect(place(0, 0, 5, 400, 17)).toBeNull();
  const boxes = Array.from({ length: 40 }, (_, i) => place(70 + (i % 3) * 38, 50 + Math.floor(i / 3) * 28, 8, 95, 17)).filter(box => box !== null);
  for (const [i, a] of boxes.entries()) for (const b of boxes.slice(i + 1))
    expect(a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top).toBe(false);
});
