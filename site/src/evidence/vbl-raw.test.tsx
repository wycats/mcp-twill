import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { App } from "../App";
import { loadTrackedEvidence } from "./adapter";

const fixtureBytes = readFileSync(
  resolve(process.cwd(), "public/evidence/vbl/v0.4.9-manifest.json"),
  "utf8",
);

describe("VBL frozen fixture evidence", () => {
  it("renders and copies the exact tracked bytes including the terminal newline", async () => {
    const evidence = loadTrackedEvidence();
    const user = userEvent.setup();
    const writeText = vi
      .spyOn(navigator.clipboard, "writeText")
      .mockResolvedValue(undefined);

    expect(fixtureBytes.endsWith("\n")).toBe(true);
    expect(evidence.vbl.frozenFixtureManifestRaw).toBe(fixtureBytes);

    render(<App evidence={evidence} />);
    const copyButton = screen.getByRole("button", {
      name: "Copy fixture JSON",
    });
    const details = copyButton.closest("details");
    expect(details).not.toBeNull();
    expect(details?.querySelector("code")?.textContent).toBe(fixtureBytes);

    await user.click(copyButton);
    expect(writeText).toHaveBeenCalledWith(fixtureBytes);
  });
});
