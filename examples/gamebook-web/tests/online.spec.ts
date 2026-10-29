import { expect, test } from "@playwright/test";

test("public short story reads host prose, verifies a lost save and restores its exact publication", async ({ page }) => {
  const errors: string[] = []; page.on("pageerror", error => errors.push(error.message));
  await page.goto("/?online");
  await expect(page).toHaveTitle("Narrata online story journey");
  await expect(page.getByText(/At dusk, Mara found a lantern/)).toBeVisible();
  await page.getByText("Simulate a save failure", { exact: true }).click();
  await page.getByRole("button", { name: "Lose the next save response", exact: true }).click();
  await page.getByRole("button", { name: "Follow the light into the forest", exact: true }).click();
  await expect(page.getByRole("button", { name: "Retry and verify save", exact: true })).toBeVisible();
  await expect(page.getByText(/At dusk, Mara found a lantern/)).toBeVisible();
  await page.getByRole("button", { name: "Retry and verify save", exact: true }).click();
  await expect(page.getByText(/Under the cedar branches/)).toBeVisible();
  await page.getByRole("button", { name: "Publish B", exact: true }).click();
  await page.getByRole("button", { name: "Reopen this journey", exact: true }).click();
  await expect(page.getByRole("button", { name: "Seek the second light", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Start a new journey", exact: true }).click();
  await page.getByRole("button", { name: "Follow the light into the forest", exact: true }).click();
  await expect(page.getByRole("button", { name: "Return home with the keeper", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Seek the second light", exact: true })).toHaveCount(0);
  await page.getByRole("button", { name: "Switch account", exact: true }).click();
  await expect(page.getByText(/Account: reader-two/)).toBeVisible();
  await expect(page.getByText(/At dusk, Mara found a lantern/)).toBeVisible();
  expect(errors).toEqual([]);
});
