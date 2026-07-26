import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { CopyButton } from "./CopyButton";

describe("CopyButton", () => {
  it("resets copy feedback when its value or label changes", async () => {
    const user = userEvent.setup();
    const { rerender } = render(
      <CopyButton value="first value" label="Copy Rust" />,
    );

    await user.click(screen.getByRole("button", { name: "Copy Rust" }));
    expect(screen.getByRole("button", { name: "Copied" })).toBeInTheDocument();

    rerender(<CopyButton value="second value" label="Copy Rust" />);
    expect(
      screen.getByRole("button", { name: "Copy Rust" }),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Copy Rust" }));
    rerender(<CopyButton value="second value" label="Copy JSON" />);
    expect(
      screen.getByRole("button", { name: "Copy JSON" }),
    ).toBeInTheDocument();
  });

  it("ignores an older clipboard write that finishes after a newer one", async () => {
    let finishFirstCopy: (() => void) | undefined;
    const writeText = vi
      .spyOn(navigator.clipboard, "writeText")
      .mockImplementationOnce(
        () =>
          new Promise<void>((resolve) => {
            finishFirstCopy = resolve;
          }),
      )
      .mockResolvedValueOnce(undefined);
    const user = userEvent.setup();
    const { rerender } = render(
      <CopyButton value="first value" label="Copy Rust" />,
    );

    await user.click(screen.getByRole("button", { name: "Copy Rust" }));
    rerender(<CopyButton value="second value" label="Copy Rust" />);
    await user.click(screen.getByRole("button", { name: "Copy Rust" }));
    expect(screen.getByRole("button", { name: "Copied" })).toBeInTheDocument();

    await act(async () => finishFirstCopy?.());
    expect(
      screen.getByRole("button", { name: "Copied" }),
    ).toBeInTheDocument();
    expect(writeText).toHaveBeenNthCalledWith(1, "first value");
    expect(writeText).toHaveBeenNthCalledWith(2, "second value");
  });
});
