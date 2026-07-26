import axe from "axe-core";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { App } from "./App";
import { findVariant, loadTrackedEvidence } from "./evidence/adapter";

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
    expect(
      screen.getByLabelText("Confirmation presentation"),
    ).toBeInTheDocument();
  });

  it("stages generated causes and consequences for the guided journey", async () => {
    const user = userEvent.setup();
    render(<App evidence={evidence} />);
    const proof = screen.getByRole("region", { name: "Guided proof" });
    const first = screen.getByRole("button", {
      name: "See the five promises",
    });
    const title = screen.getByRole("button", {
      name: "Change the title rule",
    });

    expect(proof).toHaveAttribute("data-guide-step", "none");
    await user.click(first);
    expect(first).toHaveAttribute("aria-current", "step");
    expect(first).toHaveAttribute("aria-controls", "guided-proof");
    expect(proof).toHaveAttribute("data-guide-step", "1");
    expect(
      proof.querySelectorAll("[data-proof-projection]"),
    ).toHaveLength(5);

    await user.click(title);
    expect(first).not.toHaveAttribute("aria-current");
    expect(title).toHaveAttribute("aria-current", "step");
    expect(proof).toHaveAttribute("data-guide-step", "2");
    expect(
      proof.querySelector('[data-proof-fact="fact.titleRule"]'),
    ).not.toBeNull();

    const selected = findVariant(evidence.bundle.variants, {
      ...evidence.bundle.defaults.selection,
      titleRule: "nonEmpty",
    });
    const anchor = selected.semanticAnchors.find(
      (candidate) => candidate.sourceFact === "fact.titleRule",
    )!;
    const generatedTargets = anchor.targetIds.filter((targetId) =>
      selected.comparisonTargets
        .find((target) => target.id === targetId)!
        .profiles.includes("native"),
    );
    const renderedTargets = Array.from(
      proof.querySelectorAll<HTMLElement>("[data-proof-target]"),
      (target) => target.dataset.proofTarget,
    );
    expect(renderedTargets).toEqual(generatedTargets);
    expect(
      within(proof).getAllByText("Minimum 1 character"),
    ).not.toHaveLength(0);

    await user.click(
      screen.getByRole("button", { name: "Supply private context" }),
    );
    expect(proof).toHaveAttribute("data-guide-step", "6");
    expect(
      within(proof).getByText("Raw identity serialized"),
    ).toBeInTheDocument();
    expect(within(proof).getByText("no")).toBeInTheDocument();
    expect(within(proof).getAllByText("yes")).toHaveLength(2);
  });

  it("introduces exact handwritten mismatches and restores atomically", async () => {
    const user = userEvent.setup();
    render(<App evidence={evidence} />);
    await user.click(screen.getByRole("button", { name: /Handwritten/ }));

    const check = screen.getByRole("status");
    const proof = screen.getByRole("region", { name: "Guided proof" });
    expect(check).toHaveTextContent("3 mismatches found");
    expect(within(proof).getByText("Help")).toBeInTheDocument();
    expect(within(proof).getByText("Schema")).toBeInTheDocument();
    expect(within(proof).getByText("Host")).toBeInTheDocument();
    for (const select of screen.getAllByRole("combobox")) {
      expect(select).toBeDisabled();
    }

    const variant = findVariant(
      evidence.bundle.variants,
      evidence.bundle.defaults.selection,
    );
    const canonical = new Map(
      variant.comparisonTargets
        .filter((target) => target.editable)
        .map((target) => [target.label, target.displayValue] as const),
    );
    fireEvent.change(screen.getByLabelText("Help title constraint"), {
      target: { value: canonical.get("Help title constraint") },
    });
    expect(check).toHaveTextContent("2 mismatches found");
    expect(proof).toHaveTextContent(
      "2 handwritten promises contradict the catalog.",
    );

    for (const label of [
      "Schema title constraint",
      "Host title constraint",
    ]) {
      fireEvent.change(screen.getByLabelText(label), {
        target: { value: canonical.get(label) },
      });
    }
    expect(check).toHaveTextContent(
      "all generated comparison targets agree",
    );
    expect(proof).toHaveTextContent(
      "Handwritten values match—but authority remains split.",
    );
    expect(proof).toHaveTextContent(
      "The copies match again, but each still owns an editable truth.",
    );

    await user.click(
      within(proof).getByRole("button", { name: "Restore from catalog" }),
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "all generated comparison targets agree",
    );
    expect(proof).toHaveAttribute("data-guide-step", "5");
    expect(
      within(proof).getByText(
        "Authority restored—and checked before this site ships.",
      ),
    ).toBeInTheDocument();
    expect(
      within(proof).getByText(/regenerates and byte-compares/),
    ).toBeInTheDocument();
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
    const proof = within(
      screen.getByRole("region", { name: "Guided proof" }),
    );
    expect(
      proof.getByText("One catalog operation. Two public call shapes."),
    ).toBeInTheDocument();
    expect(proof.getByText("run-write")).toBeInTheDocument();
    expect(proof.getByText("issues_create")).toBeInTheDocument();
    expect(proof.getByText("issues.create")).toBeInTheDocument();

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

  it("closes the MCP comparison when another guide chapter is selected", async () => {
    const user = userEvent.setup();
    const { container } = render(<App evidence={evidence} />);
    const proof = screen.getByRole("region", { name: "Guided proof" });
    const panel = container.querySelector<HTMLElement>(
      '[data-projection-panel="mcp"]',
    )!;
    const compare = screen.getByRole("button", {
      name: "Compare the two tool shapes",
    });

    await user.click(compare);
    expect(panel).toHaveAttribute("data-mcp-view", "comparison");

    await user.click(
      screen.getByRole("button", { name: "Change the title rule" }),
    );
    expect(proof).toHaveAttribute("data-guide-step", "2");
    expect(proof).toHaveTextContent("One source fact. Every affected promise.");
    expect(panel).toHaveAttribute("data-mcp-view", "native");

    await user.click(compare);
    expect(panel).toHaveAttribute("data-mcp-view", "comparison");
    await user.click(
      screen.getByRole("button", { name: "Restore from catalog" }),
    );
    expect(proof).toHaveAttribute("data-guide-step", "5");
    expect(proof).toHaveTextContent(
      "Authority restored—and checked before this site ships.",
    );
    expect(panel).toHaveAttribute("data-mcp-view", "native");
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
