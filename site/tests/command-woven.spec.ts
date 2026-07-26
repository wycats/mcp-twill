import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

test("completes the guided 90-second journey", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const requests: string[] = [];
  page.on("request", (request) => requests.push(request.url()));

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "A Command, Woven." })).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "The command, declared once" }),
  ).toBeVisible();

  await page
    .getByRole("heading", { name: "Change the rule once" })
    .locator("..")
    .getByRole("button", { name: "Change the title rule" })
    .click();
  await expect(page.getByLabel("Title rule")).toHaveValue("nonEmpty");

  await page
    .getByRole("heading", { name: "Behavior is part of the promise" })
    .locator("..")
    .getByRole("button", { name: "Add network access" })
    .click();
  await expect(page.getByLabel("Destination")).toHaveValue("remote");

  await page
    .getByRole("heading", { name: "Handwritten copies eventually disagree" })
    .locator("..")
    .getByRole("button", { name: "Introduce drift" })
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

  await page.getByRole("button", { name: "Compact shared lanes" }).click();
  await expect(
    page.getByRole("button", { name: "Compact shared lanes" }),
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

test("header switches share one spacing rhythm", async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "desktop header geometry");
  await page.goto("/");

  const authority = page.getByLabel("Authority mode");
  const mcp = page.getByRole("group", { name: "MCP tool shape" });
  const mcpLabel = page.getByText("MCP tool shape", { exact: true });
  const compact = page.getByRole("button", { name: "Compact shared lanes" });
  const [authorityBox, mcpBox, labelBox, compactBox] = await Promise.all([
    authority.boundingBox(),
    mcp.boundingBox(),
    mcpLabel.boundingBox(),
    compact.boundingBox(),
  ]);

  expect(authorityBox).not.toBeNull();
  expect(mcpBox).not.toBeNull();
  expect(labelBox).not.toBeNull();
  expect(compactBox).not.toBeNull();
  expect(Math.abs(authorityBox!.y - mcpBox!.y)).toBeLessThan(0.5);

  await compact.focus();
  const focus = await compact.evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      outline: Number.parseFloat(style.outlineWidth),
      offset: Number.parseFloat(style.outlineOffset),
    };
  });
  expect(
    compactBox!.y - focus.outline - focus.offset - (labelBox!.y + labelBox!.height),
  ).toBeGreaterThanOrEqual(2);
});

test("has no page-level overflow near the projection-grid breakpoint", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "desktop transition widths");

  for (const width of [
    1295,
    1250,
    1201,
    1101,
    1100,
    1060,
    961,
    960,
    940,
    901,
    900,
    899,
  ]) {
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

test("projection facts illuminate the matching Rust declaration", async ({
  page,
}) => {
  await page.goto("/");
  const target = page.getByRole("button", { name: "Help title constraint" });
  await target.hover();

  const activeTitleLines = page.locator(
    '[data-code-fact="fact.titleRule"][data-active="true"]',
  );
  await expect.poll(() => activeTitleLines.count()).toBeGreaterThan(0);
  await expect(
    page.locator(
      '[data-code-fact="fact.destination"][data-active="true"]',
    ),
  ).toHaveCount(0);
  await expect(page.locator(".token.string").first()).toBeVisible();

  const sourceLayout = await page
    .getByLabel("Generated Rust declaration")
    .evaluate((element) => ({
      whiteSpace: getComputedStyle(element).whiteSpace,
      clientWidth: element.clientWidth,
      scrollWidth: element.scrollWidth,
      clientHeight: element.clientHeight,
      scrollHeight: element.scrollHeight,
      viewportWidth: window.innerWidth,
    }));
  expect(sourceLayout.whiteSpace).toBe("pre");
  expect(sourceLayout.scrollHeight).toBe(sourceLayout.clientHeight);
  if (sourceLayout.viewportWidth >= 640) {
    expect(sourceLayout.scrollWidth).toBeLessThanOrEqual(
      sourceLayout.clientWidth + 1,
    );
  }

  await target.focus();
  await expect.poll(() => activeTitleLines.count()).toBeGreaterThan(0);

  const destinationLine = page
    .locator('[data-code-fact="fact.destination"]')
    .first();
  await destinationLine.hover();
  await expect
    .poll(() =>
      page
        .locator(
          '[data-code-fact="fact.destination"][data-active="true"]',
        )
        .count(),
    )
    .toBeGreaterThan(0);
  await page.getByRole("heading", { name: "The command, declared once" }).hover();
  await expect.poll(() => activeTitleLines.count()).toBeGreaterThan(0);
});

test("wide workbenches keep the full declaration beside its projections", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== "desktop", "wide workbench layout");
  await page.goto("/");

  const declaration = page.getByRole("region", {
    name: "The command, declared once",
  });
  const projectionArea = page
    .locator('[data-projection-panel="help"]')
    .locator("..");
  const code = page.getByLabel("Generated Rust declaration");

  const layout = await Promise.all([
    declaration.boundingBox(),
    projectionArea.boundingBox(),
    code.evaluate((element) => ({
      clientWidth: element.clientWidth,
      scrollWidth: element.scrollWidth,
      clientHeight: element.clientHeight,
      scrollHeight: element.scrollHeight,
    })),
  ]);
  const [declarationBox, projectionBox, codeBox] = layout;

  expect(declarationBox).not.toBeNull();
  expect(projectionBox).not.toBeNull();
  expect(declarationBox!.width).toBeGreaterThan(projectionBox!.width * 2);
  expect(projectionBox!.x).toBeGreaterThanOrEqual(
    declarationBox!.x + declarationBox!.width,
  );
  expect(codeBox.scrollWidth).toBeLessThanOrEqual(codeBox.clientWidth + 1);
  expect(codeBox.scrollHeight).toBe(codeBox.clientHeight);

  await page.setViewportSize({ width: 1280, height: 900 });
  await expect
    .poll(async () => {
      const source = await declaration.boundingBox();
      const projection = await projectionArea.boundingBox();
      return source && projection
        ? projection.y >= source.y + source.height
        : false;
    })
    .toBe(true);
});

test("omitted private context hover does not move its source row", async ({
  page,
}) => {
  await page.goto("/");
  const source = page.locator(
    '[data-source-fact="fact.privateContext"]',
  );
  const absence = page.locator(
    '[data-code-absence="fact.privateContext"]',
  );
  await source.scrollIntoViewIfNeeded();
  const before = await source.boundingBox();

  await source.hover();
  await expect(source).toHaveAttribute("data-active", "true");
  await expect(absence).toHaveAttribute("data-visible", "true");
  await page.waitForTimeout(250);

  const after = await source.boundingBox();
  expect(before).not.toBeNull();
  expect(after).not.toBeNull();
  expect(Math.abs(after!.y - before!.y)).toBeLessThan(0.5);
  expect(Math.abs(after!.height - before!.height)).toBeLessThan(0.5);
  await expect(source).toHaveAttribute("data-active", "true");
});

test("restore keeps focus on a mounted trigger", async ({ page }) => {
  await page.goto("/");
  const restore = page
    .getByRole("heading", { name: "Give truth one home" })
    .locator("..")
    .getByRole("button", { name: "Restore from catalog" });
  await restore.click();
  await expect(restore).toBeFocused();
});

test("chapter seven compares both MCP shapes from its full card", async ({
  page,
}) => {
  await page.goto("/");
  const chapter = page
    .getByRole("heading", { name: "One operation, two public call shapes" })
    .locator("..");
  const number = chapter.getByText("07", { exact: true });
  const compact = page.getByRole("button", { name: "Compact shared lanes" });
  const native = page.getByRole("button", { name: "Native direct tool" });
  const mcpPanel = page.locator('[data-projection-panel="mcp"]');

  const clickChapterNumber = async () => {
    await number.scrollIntoViewIfNeeded();
    const bounds = await number.boundingBox();
    expect(bounds).not.toBeNull();
    const point = {
      x: bounds!.x + bounds!.width / 2,
      y: bounds!.y + bounds!.height / 2,
    };
    const hitTarget = await page.evaluate(
      ({ x, y }) => document.elementFromPoint(x, y)?.closest("button")?.textContent,
      point,
    );
    expect(hitTarget).toMatch(
      /Compare the two tool shapes|Return to native tool/,
    );
    await page.mouse.click(point.x, point.y);
  };

  await clickChapterNumber();
  const closeComparison = page.getByRole("button", {
    name: "Close generated comparison",
  });
  await expect(closeComparison).toHaveAttribute("aria-pressed", "true");
  await expect(compact).toHaveAttribute("aria-pressed", "false");
  await expect(native).toHaveAttribute("aria-pressed", "false");
  await expect(mcpPanel).toHaveAttribute("data-mcp-view", "comparison");
  const comparison = mcpPanel.getByLabel(
    "Compact and Native MCP tool comparison",
  );
  await expect(comparison).toBeVisible();
  const accessibility = await new AxeBuilder({ page }).analyze();
  expect(accessibility.violations).toEqual([]);
  await expect(
    comparison.getByText(
      /“Compact” describes how the tool surface scales across many commands/,
    ),
  ).toBeVisible();
  await expect(
    comparison.getByText("Shared effect lanes"),
  ).toBeVisible();
  await expect(
    comparison.getByText("Direct operation tool"),
  ).toBeVisible();
  await expect(
    comparison.getByText("run-write", { exact: true }),
  ).toBeVisible();
  await expect(comparison.getByText("issues_create").first()).toBeVisible();
  await expect(comparison.getByText("issues.create")).toBeVisible();

  await clickChapterNumber();
  await expect(
    page.getByRole("button", { name: "Compare both generated shapes" }),
  ).toHaveAttribute("aria-pressed", "false");
  await expect(native).toHaveAttribute("aria-pressed", "true");
  await expect(mcpPanel).toHaveAttribute("data-mcp-view", "native");
  await expect(comparison).toBeHidden();
});

test("request step seven exposes its result state", async ({ page }) => {
  await page.goto("/");
  const resultStep = page.getByRole("button", { name: "7 Result / task" });

  await resultStep.click();

  await expect(resultStep).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByText("Step 7 of 7 · Result / task")).toBeVisible();
  await expect(
    page.getByText(
      "This specimen returns immediately; task support remains optional.",
    ),
  ).toBeVisible();
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

  await page.getByRole("button", { name: "Compact shared lanes" }).click();

  await expect(surface).toHaveText(beforeSurface ?? "");
  await expect(
    page
      .getByRole("article")
      .filter({ has: page.getByText("The catalog selects one operation.") }),
  ).toHaveText(beforeTrace ?? "");
  await expect(
    page
      .locator('[data-projection-panel="mcp"]')
      .getByText(/compact · shared effect lanes · catalog-derived/),
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

  await page
    .getByRole("button", { name: "Compare both generated shapes" })
    .click();
  await expect(page.getByRole("tab", { name: "MCP" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  const mcpPanel = page.locator('[data-projection-panel="mcp"]');
  await expect(mcpPanel).toBeVisible();
  await expect(mcpPanel).toHaveAttribute("data-mcp-view", "comparison");
  await expect(
    mcpPanel.getByLabel("Compact and Native MCP tool comparison"),
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
