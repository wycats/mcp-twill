import axe from "axe-core";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { App } from "./App";
import { loadTrackedEvidence } from "./evidence/adapter";

const evidence = loadTrackedEvidence();

describe("A Command, Woven", () => {
  it("renders one declaration and all five synchronized projections", () => {
    render(<App evidence={evidence} />);
    expect(
      screen.getByRole("heading", { name: "The command, declared once" }),
    ).toBeInTheDocument();
    for (const name of ["Help", "Schema", "MCP", "Confirmation", "Host"]) {
      expect(screen.getByRole("heading", { name })).toBeInTheDocument();
    }
    expect(
      screen.getByText(
        "Illustrative host rendering — layout is site-owned; values are Twill-generated.",
      ),
    ).toBeInTheDocument();
  });

  it("introduces exact handwritten mismatches and restores atomically", async () => {
    const user = userEvent.setup();
    render(<App evidence={evidence} />);
    await user.click(screen.getByRole("button", { name: /Handwritten/ }));

    const check = screen.getByRole("status");
    expect(check).toHaveTextContent("3 mismatches found");
    expect(within(check).getByText(/help/)).toBeInTheDocument();
    expect(within(check).getByText(/schema/)).toBeInTheDocument();
    expect(within(check).getByText(/host/)).toBeInTheDocument();
    for (const select of screen.getAllByRole("combobox")) {
      expect(select).toBeDisabled();
    }

    await user.click(
      within(check).getByRole("button", { name: "Restore from catalog" }),
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "all evidence-declared comparison targets agree",
    );
    for (const select of screen.getAllByRole("combobox")) {
      expect(select).toBeEnabled();
    }
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /Derived/ })).toHaveFocus(),
    );
  });

  it("switches authentic compact and native MCP projections", async () => {
    const user = userEvent.setup();
    render(<App evidence={evidence} />);
    expect(
      screen.getByRole("group", { name: "MCP tool shape" }),
    ).toBeInTheDocument();
    const native = screen.getByRole("button", {
      name: "Native direct tool",
    });
    const compact = screen.getByRole("button", {
      name: "Compact shared lanes",
    });
    expect(native).toHaveAttribute("aria-pressed", "true");
    expect(
      screen.getByRole("button", { name: "Native MCP title constraint" }),
    ).toBeInTheDocument();
    await user.click(compact);
    expect(compact).toHaveAttribute("aria-pressed", "true");
    expect(
      screen.queryByRole("button", { name: "Native MCP title constraint" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Compact effect-lane tool" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/Compact MCP projection selected/),
    ).toBeInTheDocument();
  });

  it("uses the seventh guide step to compare both MCP profiles", async () => {
    const user = userEvent.setup();
    const { container } = render(<App evidence={evidence} />);
    const initialCompare = screen.getByRole("button", {
      name: "Compare both generated shapes",
    });
    const compact = screen.getByRole("button", {
      name: "Compact shared lanes",
    });
    const native = screen.getByRole("button", {
      name: "Native direct tool",
    });

    await user.click(
      screen.getByRole("button", {
        name: "Compare the two tool shapes",
      }),
    );
    expect(
      screen.getByRole("button", {
        name: "Close generated comparison",
      }),
    ).toHaveAttribute("aria-pressed", "true");
    expect(compact).toHaveAttribute("aria-pressed", "false");
    expect(native).toHaveAttribute("aria-pressed", "false");

    const panel = container.querySelector<HTMLElement>(
      '[data-projection-panel="mcp"]',
    )!;
    expect(panel).toHaveAttribute("data-mcp-view", "comparison");
    const comparison = within(
      screen.getByLabelText("Compact and Native MCP tool comparison"),
    );
    expect(
      comparison.getByText(
        /“Compact” describes how the tool surface scales across many commands/,
      ),
    ).toBeInTheDocument();
    expect(
      comparison.getByText("Shared effect lanes"),
    ).toBeInTheDocument();
    expect(
      comparison.getByText("Direct operation tool"),
    ).toBeInTheDocument();
    expect(comparison.getByText("run-write")).toBeInTheDocument();
    expect(comparison.getAllByText("issues_create")).not.toHaveLength(0);
    expect(comparison.getByText("issues.create")).toBeInTheDocument();

    await user.click(native);
    expect(initialCompare).toHaveAccessibleName("Compare both generated shapes");
    expect(
      screen.getByRole("button", {
        name: "Compare both generated shapes",
      }),
    ).toHaveAttribute("aria-pressed", "false");
    expect(native).toHaveAttribute("aria-pressed", "true");
    expect(panel).toHaveAttribute("data-mcp-view", "native");
    expect(
      screen.queryByLabelText("Compact and Native MCP tool comparison"),
    ).not.toBeInTheDocument();
  });

  it("makes the selected request-microscope step explicit", async () => {
    const user = userEvent.setup();
    render(<App evidence={evidence} />);
    const resultStep = screen.getByRole("button", {
      name: "7 Result / task",
    });

    await user.click(resultStep);

    expect(resultStep).toHaveAttribute("aria-pressed", "true");
    expect(resultStep).toHaveAttribute("aria-controls", "trace-detail");
    expect(screen.getByText("Step 7 of 7 · Result / task")).toBeInTheDocument();
    expect(
      screen.getByText(
        "This specimen returns immediately; task support remains optional.",
      ),
    ).toBeInTheDocument();
  });

  it("highlights the authoritative Rust from projection hover and focus", async () => {
    const user = userEvent.setup();
    const { container } = render(<App evidence={evidence} />);
    const target = screen.getByRole("button", {
      name: "Native MCP title constraint",
    });
    const activeCode = () =>
      container.querySelectorAll(
        '[data-code-fact="fact.titleRule"][data-active="true"]',
      );

    await user.hover(target);
    expect(activeCode().length).toBeGreaterThan(0);
    expect(
      container.querySelectorAll(
        '[data-code-fact="fact.destination"][data-active="true"]',
      ),
    ).toHaveLength(0);

    await user.unhover(target);
    expect(activeCode()).toHaveLength(0);

    await user.click(target);
    await user.unhover(target);
    expect(activeCode().length).toBeGreaterThan(0);
    await user.tab();
    expect(activeCode()).toHaveLength(0);
  });

  it("restores a focused relationship after a different fact is hovered", async () => {
    const user = userEvent.setup();
    const { container } = render(<App evidence={evidence} />);
    const focusedTarget = screen.getByRole("button", {
      name: "Native MCP title constraint",
    });
    const hoveredLine = container.querySelector<HTMLElement>(
      '[data-code-fact="fact.destination"]',
    )!;

    fireEvent.focus(focusedTarget);
    expect(
      container.querySelectorAll(
        '[data-code-fact="fact.titleRule"][data-active="true"]',
      ).length,
    ).toBeGreaterThan(0);

    await user.hover(hoveredLine);
    expect(
      container.querySelectorAll(
        '[data-code-fact="fact.destination"][data-active="true"]',
      ).length,
    ).toBeGreaterThan(0);

    await user.unhover(hoveredLine);
    expect(
      container.querySelectorAll(
        '[data-code-fact="fact.titleRule"][data-active="true"]',
      ).length,
    ).toBeGreaterThan(0);
  });

  it("reveals an omitted source fact without mounting layout-changing content", async () => {
    const user = userEvent.setup();
    render(<App evidence={evidence} />);
    const privateContext = screen.getByRole("button", {
      name: /fact\.privateContext Private context/,
    });
    const absence = document.querySelector<HTMLElement>(
      '[data-code-absence="fact.privateContext"]',
    )!;

    expect(absence).toHaveAttribute("aria-hidden", "true");
    await user.hover(privateContext);
    expect(absence).toHaveAttribute("aria-hidden", "false");
    expect(absence).toHaveTextContent(
      "no declaration line is emitted in this variant",
    );
    await user.unhover(privateContext);
    expect(absence).toHaveAttribute("aria-hidden", "true");
  });

  it("supports arrow navigation through the mobile projection tablist", async () => {
    const user = userEvent.setup();
    render(<App evidence={evidence} />);
    const help = screen.getByRole("tab", { name: "Help", hidden: true });
    help.focus();
    await user.keyboard("{ArrowRight}");
    expect(
      screen.getByRole("tab", { name: "Schema", hidden: true }),
    ).toHaveFocus();
    expect(
      screen.getByRole("tab", { name: "Schema", hidden: true }),
    ).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("exposes request stages, private-context checks, and provenance", () => {
    render(<App evidence={evidence} />);
    expect(
      screen.getByRole("heading", { name: /Select → bind/ }),
    ).toBeInTheDocument();
    expect(
      screen.getAllByText(/Stage taxonomy: essay-authored/),
    ).not.toHaveLength(0);
    expect(
      screen.getByRole("heading", {
        name: "Useful to the decision, absent from the API",
      }),
    ).toBeInTheDocument();
    expect(screen.getByText("A frozen adoption case")).toBeInTheDocument();
  });

  it("has no axe violations in baseline and drift states", async () => {
    const user = userEvent.setup();
    const { container } = render(<App evidence={evidence} />);
    const options = { rules: { "color-contrast": { enabled: false } } };
    expect((await axe.run(container, options)).violations).toEqual([]);
    await user.click(screen.getByRole("button", { name: /Handwritten/ }));
    expect((await axe.run(container, options)).violations).toEqual([]);
  });
});
