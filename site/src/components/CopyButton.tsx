import { useState } from "react";
import styles from "./CopyButton.module.css";

interface CopyButtonProps {
  value: string;
  label?: string;
}

export function CopyButton({ value, label = "Copy" }: CopyButtonProps) {
  const [status, setStatus] = useState(label);

  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
      setStatus("Copied");
    } catch {
      setStatus("Copy unavailable");
    }
  }

  return (
    <button className={styles.button} type="button" onClick={copy}>
      {status}
    </button>
  );
}
