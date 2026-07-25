import { useRef, useState } from "react";
import styles from "./CopyButton.module.css";

interface CopyButtonProps {
  value: string;
  label?: string;
}

interface CopyFeedback {
  value: string;
  label: string;
  message: "Copied" | "Copy unavailable";
}

export function CopyButton({ value, label = "Copy" }: CopyButtonProps) {
  const [feedback, setFeedback] = useState<CopyFeedback | null>(null);
  const latestRequestId = useRef(0);
  const status =
    feedback?.value === value && feedback.label === label
      ? feedback.message
      : label;

  async function copy() {
    const requestId = ++latestRequestId.current;
    try {
      await navigator.clipboard.writeText(value);
      if (requestId === latestRequestId.current) {
        setFeedback({ value, label, message: "Copied" });
      }
    } catch {
      if (requestId === latestRequestId.current) {
        setFeedback({ value, label, message: "Copy unavailable" });
      }
    }
  }

  return (
    <button className={styles.button} type="button" onClick={copy}>
      {status}
    </button>
  );
}
