import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { EvidenceFailure } from "./components/EvidenceFailure";
import { loadTrackedEvidence } from "./evidence/adapter";
import "./global.css";

const root = createRoot(document.getElementById("root")!);

try {
  const evidence = loadTrackedEvidence();
  root.render(
    <StrictMode>
      <App evidence={evidence} />
    </StrictMode>,
  );
} catch (error) {
  root.render(<EvidenceFailure error={error} />);
}
