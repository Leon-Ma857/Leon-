import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

export interface SlidingSegmentOption<T extends string> {
  value: T;
  label: React.ReactNode;
  title?: string;
  disabled?: boolean;
}

export interface SlidingSegmentedProps<T extends string> {
  options: readonly SlidingSegmentOption<T>[];
  value: T;
  onChange: (value: T) => void;
  ariaLabel?: string;
  size?: "xs" | "sm" | "md";
  className?: string;
}

interface ThumbBox {
  left: number;
  width: number;
  ready: boolean;
}

export function SlidingSegmented<T extends string>({
  options,
  value,
  onChange,
  ariaLabel,
  size = "md",
  className = "",
}: SlidingSegmentedProps<T>) {
  const railRef = useRef<HTMLDivElement | null>(null);
  const btnRefs = useRef<Record<string, HTMLButtonElement | null>>({});
  const prevBoxRef = useRef<{ left: number; width: number } | null>(null);
  const prevValueRef = useRef<T>(value);
  const timerRef = useRef<number | null>(null);

  const [thumb, setThumb] = useState<ThumbBox>({ left: 0, width: 0, ready: false });
  const [animate, setAnimate] = useState(false);

  const measureActive = useCallback(
    (stretch: boolean) => {
      const btn = btnRefs.current[value];
      if (!btn) return;
      const nextLeft = btn.offsetLeft;
      const nextWidth = btn.offsetWidth;
      if (nextWidth <= 0) return;

      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current);
        timerRef.current = null;
      }

      const reduced =
        typeof window !== "undefined" &&
        window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
      const prev = prevBoxRef.current;

      if (
        stretch &&
        !reduced &&
        prev &&
        (Math.abs(prev.left - nextLeft) > 1 || Math.abs(prev.width - nextWidth) > 1)
      ) {
        setAnimate(true);
        const spanLeft = Math.min(prev.left, nextLeft);
        const spanRight = Math.max(prev.left + prev.width, nextLeft + nextWidth);
        const spanWidth = Math.min(
          spanRight - spanLeft,
          Math.max(prev.width, nextWidth) * 1.42
        );
        const midLeft =
          nextLeft > prev.left
            ? nextLeft - (spanWidth - nextWidth) * 0.52
            : nextLeft;

        setThumb({ left: midLeft, width: spanWidth, ready: true });
        timerRef.current = window.setTimeout(() => {
          timerRef.current = null;
          setThumb({ left: nextLeft, width: nextWidth, ready: true });
          prevBoxRef.current = { left: nextLeft, width: nextWidth };
        }, 72);
      } else {
        setThumb({ left: nextLeft, width: nextWidth, ready: true });
        prevBoxRef.current = { left: nextLeft, width: nextWidth };
      }
    },
    [value]
  );

  useLayoutEffect(() => {
    const changed = prevValueRef.current !== value;
    prevValueRef.current = value;
    measureActive(changed);
  }, [value, options.length, measureActive]);

  useEffect(() => {
    const rail = railRef.current;
    if (!rail || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => {
      measureActive(false);
    });
    ro.observe(rail);
    return () => {
      ro.disconnect();
      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current);
      }
    };
  }, [measureActive]);

  return (
    <div
      ref={railRef}
      role="tablist"
      aria-label={ariaLabel}
      className={`v3-seg-rail v3-seg-${size}${animate ? " is-animated" : ""}${className ? ` ${className}` : ""}`}
    >
      <div
        className="v3-seg-thumb"
        aria-hidden="true"
        style={{
          opacity: thumb.ready ? 1 : 0,
          width: `${thumb.width}px`,
          transform: `translate3d(${thumb.left}px, 0, 0)`,
        }}
      />
      {options.map((opt) => {
        const active = opt.value === value;
        return (
          <button
            key={opt.value}
            ref={(el) => {
              btnRefs.current[opt.value] = el;
            }}
            type="button"
            role="tab"
            aria-selected={active}
            disabled={opt.disabled}
            title={opt.title}
            className={`v3-seg-tab${active ? " active" : ""}`}
            onClick={() => {
              if (!opt.disabled && opt.value !== value) {
                onChange(opt.value);
              }
            }}
          >
            {opt.label}
          </button>
        );
      })}
    </div>
  );
}
