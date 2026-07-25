import styles from "./EvidenceFailure.module.css";

export function EvidenceFailure({ error }: { error: unknown }) {
  const message =
    error instanceof Error ? error.message : "Unknown evidence error";
  return (
    <main className={styles.failure}>
      <p>Generated evidence could not be loaded.</p>
      <pre>
        <code>{message}</code>
      </pre>
    </main>
  );
}
