import { useMemo } from "react";
import { Highlight, type PrismTheme } from "prism-react-renderer";
import type { DeclarationEvidence } from "../evidence/types";
import styles from "./DeclarationCode.module.css";

const twillRustTheme: PrismTheme = {
  plain: {
    color: "var(--ink)",
    backgroundColor: "transparent",
  },
  styles: [
    {
      types: ["comment"],
      style: {
        color: "var(--muted)",
        fontStyle: "italic",
      },
    },
    {
      types: ["keyword"],
      style: {
        color: "var(--teal)",
        fontWeight: "600",
      },
    },
    {
      types: ["string", "char"],
      style: {
        color: "var(--rust)",
      },
    },
    {
      types: ["function", "function-definition"],
      style: {
        color: "var(--teal)",
      },
    },
    {
      types: ["class-name", "type-definition", "constant", "macro", "property"],
      style: {
        color: "var(--rust)",
        fontWeight: "500",
      },
    },
    {
      types: ["number", "boolean"],
      style: {
        color: "var(--success)",
      },
    },
    {
      types: ["operator", "punctuation"],
      style: {
        color: "var(--muted)",
      },
    },
  ],
};

interface DeclarationCodeProps {
  declaration: DeclarationEvidence;
  activeFactId: string | null;
  onHoverFact: (factId: string | null) => void;
}

export function DeclarationCode({
  declaration,
  activeFactId,
  onHoverFact,
}: DeclarationCodeProps) {
  const factByLine = useMemo(() => {
    const lines = new Map<number, string>();
    for (const fact of declaration.facts) {
      for (const range of fact.codeRanges) {
        for (
          let lineNumber = range.startLine;
          lineNumber <= range.endLine;
          lineNumber += 1
        ) {
          lines.set(lineNumber, fact.id);
        }
      }
    }
    return lines;
  }, [declaration.facts]);
  return (
    <div className={styles.frame}>
      <Highlight
        code={declaration.text}
        language="rust"
        theme={twillRustTheme}
      >
        {({ className, style, tokens, getLineProps, getTokenProps }) => (
          <pre
            className={`${className} ${styles.pre}`}
            style={style}
            tabIndex={0}
            aria-label="Generated Rust declaration"
          >
            <code>
              {tokens.map((line, lineIndex) => {
                const lineNumber = lineIndex + 1;
                const factId = factByLine.get(lineNumber);
                const active = factId !== undefined && activeFactId === factId;
                const lineProps = getLineProps({ line });
                return (
                  <span
                    {...lineProps}
                    key={lineNumber}
                    className={[
                      lineProps.className,
                      styles.line,
                      factId ? styles.related : "",
                      active ? styles.active : "",
                    ].join(" ")}
                    data-line-number={lineNumber}
                    data-code-fact={factId}
                    data-active={active}
                    onPointerEnter={
                      factId ? () => onHoverFact(factId) : undefined
                    }
                    onPointerLeave={
                      factId ? () => onHoverFact(null) : undefined
                    }
                  >
                    <span className={styles.lineContent}>
                      {line.map((token, tokenIndex) => (
                        <span
                          {...getTokenProps({ token })}
                          key={`${lineNumber}:${tokenIndex}`}
                        />
                      ))}
                    </span>
                  </span>
                );
              })}
            </code>
          </pre>
        )}
      </Highlight>
    </div>
  );
}
