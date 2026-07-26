import { fireEvent, render, screen } from "@testing-library/react";
import { findVariant, loadTrackedEvidence } from "../evidence/adapter";
import { DeclarationCode } from "./DeclarationCode";

const { bundle } = loadTrackedEvidence();
const declaration = findVariant(
  bundle.variants,
  bundle.defaults.selection,
).declaration;

describe("DeclarationCode", () => {
  it("syntax-highlights canonical Rust without inferring semantic ranges", () => {
    const { container } = render(
      <DeclarationCode
        declaration={declaration}
        activeFactId="fact.titleRule"
        onHoverFact={() => undefined}
      />,
    );

    expect(
      screen.getByLabelText("Generated Rust declaration"),
    ).toBeInTheDocument();
    expect(container.querySelector(".token.string")).not.toBeNull();
    expect(container.querySelector(".token.function")).not.toBeNull();

    const titleFact = declaration.facts.find(
      (fact) => fact.id === "fact.titleRule",
    )!;
    const expectedActiveLines = titleFact.codeRanges.reduce(
      (total, range) => total + range.endLine - range.startLine + 1,
      0,
    );
    expect(
      container.querySelectorAll(
        '[data-code-fact="fact.titleRule"][data-active="true"]',
      ),
    ).toHaveLength(expectedActiveLines);
    expect(
      container.querySelectorAll(
        '[data-code-fact="fact.destination"][data-active="true"]',
      ),
    ).toHaveLength(0);
  });

  it("activates rendered facts without inventing lines for omitted facts", () => {
    const onActivateFact = vi.fn();
    const { container, rerender } = render(
      <DeclarationCode
        declaration={declaration}
        activeFactId={null}
        onHoverFact={onActivateFact}
      />,
    );

    const destinationLine = container.querySelector(
      '[data-code-fact="fact.destination"]',
    )!;
    fireEvent.pointerEnter(destinationLine);
    expect(onActivateFact).toHaveBeenLastCalledWith("fact.destination");
    fireEvent.pointerLeave(destinationLine);
    expect(onActivateFact).toHaveBeenLastCalledWith(null);

    rerender(
      <DeclarationCode
        declaration={declaration}
        activeFactId="fact.privateContext"
        onHoverFact={onActivateFact}
      />,
    );
    expect(
      container.querySelectorAll(
        '[data-code-fact="fact.privateContext"][data-active="true"]',
      ),
    ).toHaveLength(0);
  });
});
