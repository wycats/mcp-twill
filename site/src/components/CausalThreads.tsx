import { useLayoutEffect, useState, type RefObject } from "react";
import type { SemanticAnchor } from "../evidence/types";
import styles from "./CausalThreads.module.css";

interface CausalThreadsProps {
  containerRef: RefObject<HTMLDivElement | null>;
  anchors: SemanticAnchor[];
  activeFactId: string | null;
  mismatchedTargetIds: ReadonlySet<string>;
}

interface ThreadPath {
  id: string;
  factId: string;
  targetId: string;
  d: string;
}

export function CausalThreads({
  containerRef,
  anchors,
  activeFactId,
  mismatchedTargetIds,
}: CausalThreadsProps) {
  const [geometry, setGeometry] = useState({
    width: 1,
    height: 1,
    paths: [] as ThreadPath[],
  });

  useLayoutEffect(() => {
    let observer: ResizeObserver | null = null;
    let container: HTMLDivElement | null = null;

    const measure = () => {
      if (!container) return;
      const bounds = container.getBoundingClientRect();
      const sources = new Map(
        Array.from(
          container.querySelectorAll<HTMLElement>("[data-source-fact]"),
        ).map((element) => [element.dataset.sourceFact ?? "", element]),
      );
      const targets = new Map(
        Array.from(
          container.querySelectorAll<HTMLElement>("[data-comparison-target]"),
        ).map((element) => [element.dataset.comparisonTarget ?? "", element]),
      );
      const paths: ThreadPath[] = [];

      for (const anchor of anchors) {
        const source = sources.get(anchor.sourceFact);
        if (!source) continue;
        const sourceBounds = source.getBoundingClientRect();
        for (const targetId of anchor.targetIds) {
          const target = targets.get(targetId);
          if (!target || target.offsetParent === null) continue;
          const targetBounds = target.getBoundingClientRect();
          const startX = sourceBounds.right - bounds.left;
          const startY =
            sourceBounds.top + sourceBounds.height / 2 - bounds.top;
          const endX = targetBounds.left - bounds.left;
          const endY =
            targetBounds.top + targetBounds.height / 2 - bounds.top;
          const bend = Math.max(28, Math.abs(endX - startX) * 0.42);
          paths.push({
            id: `${anchor.id}:${targetId}`,
            factId: anchor.sourceFact,
            targetId,
            d: `M ${startX} ${startY} C ${startX + bend} ${startY}, ${endX - bend} ${endY}, ${endX} ${endY}`,
          });
        }
      }

      setGeometry({
        width: Math.max(1, bounds.width),
        height: Math.max(1, bounds.height),
        paths,
      });
    };

    // A child layout effect can run before React assigns the surrounding
    // element's ref. The next animation frame sees the complete committed
    // specimen and avoids permanently freezing the SVG at its 1×1 fallback.
    const frame = requestAnimationFrame(() => {
      container = containerRef.current;
      if (!container) return;
      measure();
      observer = new ResizeObserver(measure);
      observer.observe(container);
      for (const element of container.querySelectorAll<HTMLElement>(
        "[data-source-fact], [data-comparison-target]",
      )) {
        observer.observe(element);
      }
      window.addEventListener("resize", measure);
    });

    return () => {
      cancelAnimationFrame(frame);
      observer?.disconnect();
      window.removeEventListener("resize", measure);
    };
  }, [anchors, containerRef]);

  return (
    <svg
      className={styles.svg}
      viewBox={`0 0 ${geometry.width} ${geometry.height}`}
      preserveAspectRatio="none"
      aria-hidden="true"
    >
      {geometry.paths.map((path) => (
        <path
          key={path.id}
          d={path.d}
          data-thread-fact={path.factId}
          data-thread-target={path.targetId}
          data-active={activeFactId === path.factId}
          data-drifted={mismatchedTargetIds.has(path.targetId)}
          className={[
            styles.thread,
            activeFactId === path.factId ? styles.active : "",
            mismatchedTargetIds.has(path.targetId) ? styles.drifted : "",
          ].join(" ")}
        />
      ))}
    </svg>
  );
}
