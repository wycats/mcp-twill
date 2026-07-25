import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

test("completes the guided 90-second journey", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const requests: string[] = [];
  page.on("request", (request) => requests.push(request.url()));

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "A Command, Woven." })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Rust declaration" })).toBeVisible();

  await page
    .getByRole("heading", { name: "A constraint travels" })
    .locator("..")
    .getByRole("button", { name: "Show this" })
    .click();
  await expect(page.getByLabel("Title rule")).toHaveValue("nonEmpty");

  await page
    .getByRole("heading", { name: "Behavior travels too" })
    .locator("..")
    .getByRole("button", { name: "Show this" })
    .click();
  await expect(page.getByLabel("Destination")).toHaveValue("remote");

  await page
    .getByRole("heading", { name: "Drift is the bug" })
    .locator("..")
    .getByRole("button", { name: "Show this" })
    .click();
  await expect(page.getByRole("status")).toContainText("3 mismatches found");

  await page
    .getByRole("status")
    .getByRole("button", { name: "Restore from catalog" })
    .click();
  await expect(page.getByRole("status")).toContainText(
    "all evidence-declared comparison targets agree",
  );
  await expect(page.getByRole("button", { name: /Derived/ })).toBeFocused();

  await page.getByRole("button", { name: "Compact", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Compact", exact: true }),
  ).toHaveAttribute(
    "aria-pressed",
    "true",
  );

  await page.getByRole("button", { name: "Copy Rust" }).click();
  await expect(page.getByRole("button", { name: "Copied" })).toBeVisible();
  await page.getByText("Declaration provenance").click();
  await expect(
    page.getByRole("link", { name: /site_specimen/ }).first(),
  ).toBeVisible();

  const accessibility = await new AxeBuilder({ page }).analyze();
  expect(accessibility.violations).toEqual([]);

  const externalRequests = requests.filter(
    (url) => new URL(url).origin !== "http://127.0.0.1:4173",
  );
  expect(externalRequests).toEqual([]);
});

test("is keyboard-operable", async ({ page }, testInfo) => {
  await page.goto("/");
  await page.getByRole("button", { name: /Handwritten/ }).focus();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("status")).toContainText("3 mismatches");
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.press("Tab");
  await expect(page.getByRole("button", { name: /Handwritten/ })).toBeFocused();

  if (testInfo.project.name === "mobile") {
    const help = page.getByRole("tab", { name: "Help" });
    await help.focus();
    await page.keyboard.press("ArrowRight");
    await expect(page.getByRole("tab", { name: "Schema" })).toBeFocused();
  }
});

test("has no page-level overflow at supported breakpoints", async ({ page }) => {
  await page.goto("/");
  const dimensions = await page.evaluate(() => ({
    client: document.documentElement.clientWidth,
    scroll: document.documentElement.scrollWidth,
    offenders: Array.from(document.querySelectorAll<HTMLElement>("body *"))
      .map((element) => {
        const bounds = element.getBoundingClientRect();
        return {
          tag: element.tagName.toLowerCase(),
          className: element.className,
          parentClassName: element.parentElement?.className ?? "",
          text: element.textContent?.slice(0, 80) ?? "",
          right: Math.round(bounds.right),
          width: Math.round(bounds.width),
        };
      })
      .filter(({ right }) => right > document.documentElement.clientWidth + 1)
      .slice(0, 8),
  }));
  expect(
    dimensions.scroll,
    JSON.stringify(dimensions.offenders),
  ).toBeLessThanOrEqual(dimensions.client);
});

test("has no page-level overflow near the projection-grid breakpoint", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "desktop transition widths");

  for (const width of [1295, 1250, 1201]) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto("/");

    const dimensions = await page.evaluate(() => ({
      client: document.documentElement.clientWidth,
      scroll: document.documentElement.scrollWidth,
    }));

    expect(
      dimensions.scroll,
      `page overflowed at ${width}px`,
    ).toBeLessThanOrEqual(dimensions.client);
  }
});

test("desktop causal threads survive resize and activate generated facts", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "desktop thread geometry");
  await page.goto("/");
  const paths = page.locator('svg[aria-hidden="true"] path[data-thread-fact]');
  await expect.poll(() => paths.count()).toBeGreaterThan(0);
  await page.setViewportSize({ width: 1320, height: 900 });
  await expect
    .poll(async () =>
      paths.evaluateAll((elements) =>
        elements.every((element) => {
          const d = element.getAttribute("d");
          return Boolean(d && d.length > 8);
        }),
      ),
    )
    .toBe(true);

  await page.getByLabel("Title rule").selectOption("nonEmpty");
  await expect
    .poll(() =>
      page
        .locator(
          'path[data-thread-fact="fact.titleRule"][data-active="true"]',
        )
        .count(),
    )
    .toBeGreaterThan(0);

  const activePath = page
    .locator('path[data-thread-fact="fact.titleRule"][data-active="true"]')
    .first();
  const activeStyle = await activePath.evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      opacity: Number.parseFloat(style.opacity),
      stroke: style.stroke,
      strokeWidth: Number.parseFloat(style.strokeWidth),
    };
  });
  expect(activeStyle.opacity).toBeGreaterThanOrEqual(0.9);
  expect(activeStyle.stroke).toBe("rgb(31, 109, 104)");
  expect(activeStyle.strokeWidth).toBeGreaterThanOrEqual(2);
});

test("restore keeps focus on a mounted trigger", async ({ page }) => {
  await page.goto("/");
  const restore = page
    .getByRole("heading", { name: "Restore authority" })
    .locator("..")
    .getByRole("button", { name: "Restore from catalog" });
  await restore.click();
  await expect(restore).toBeFocused();
});

test("MCP profile switching does not rewrite the captured invocation", async ({
  page,
}) => {
  await page.goto("/");
  const surface = page.getByTestId("microscope-surface");
  const beforeSurface = await surface.textContent();
  const beforeTrace = await page
    .getByRole("article")
    .filter({ has: page.getByText("The catalog selects one operation.") })
    .textContent();

  await page.getByRole("button", { name: "Compact", exact: true }).click();

  await expect(surface).toHaveText(beforeSurface ?? "");
  await expect(
    page
      .getByRole("article")
      .filter({ has: page.getByText("The catalog selects one operation.") }),
  ).toHaveText(beforeTrace ?? "");
  await expect(
    page
      .locator('[data-projection-panel="mcp"]')
      .getByText(/compact · catalog-derived/),
  ).toBeVisible();
});

test("mobile tabs replace causal paths with origin labels", async ({ page }) => {
  test.skip(test.info().project.name !== "mobile", "mobile-only assertion");
  await page.goto("/");
  await expect(page.getByRole("tablist", { name: "Generated projections" })).toBeVisible();
  await page.getByRole("tab", { name: "Schema" }).click();
  await expect(page.locator('[data-projection-panel="schema"]')).toBeVisible();
  await expect(page.locator('[data-projection-panel="help"]')).toBeHidden();
  await expect(page.locator('svg[aria-hidden="true"]')).toBeHidden();
  await expect(
    page
      .locator('[data-projection-panel="schema"]')
      .getByText(/Origin: declaration fact/)
      .first(),
  ).toBeVisible();
});

test("reduced motion removes interactive transitions", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/");
  const transition = await page
    .getByRole("status")
    .evaluate((element) => getComputedStyle(element).transitionDuration);
  const longest = Math.max(
    ...transition.split(",").map((part) => Number.parseFloat(part)),
  );
  expect(longest).toBeLessThanOrEqual(0.00001);
});
