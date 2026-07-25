import axe from "axe-core";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { App } from "./App";
import { loadTrackedEvidence } from "./evidence/adapter";

const evidence = loadTrackedEvidence();

describe("A Command, Woven", () => {
  it("renders one declaration and all five synchronized projections", () => {
    render(<App evidence={evidence} />);
    expect(
      screen.getByRole("heading", { name: "Rust declaration" }),
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
    const native = screen.getByRole("button", { name: "Native" });
    const compact = screen.getByRole("button", { name: "Compact" });
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
