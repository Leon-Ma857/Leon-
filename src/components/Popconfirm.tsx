import { useEffect, useRef, useState, type ReactNode } from "react";

export interface PopconfirmProps {
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  danger?: boolean;
  onConfirm: () => void | Promise<void>;
  children: (triggerProps: { onClick: () => void; "aria-expanded": boolean }) => ReactNode;
}

export function Popconfirm({
  message,
  confirmLabel = "确认",
  cancelLabel = "取消",
  danger = true,
  onConfirm,
  children,
}: PopconfirmProps) {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", onPointerDown);
    window.addEventListener("keydown", onKeyDown, true);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown, true);
    };
  }, [open]);

  const handleConfirm = async () => {
    setOpen(false);
    await onConfirm();
  };

  return (
    <div className="popconfirm-wrap" ref={wrapRef}>
      {children({
        onClick: () => setOpen((v) => !v),
        "aria-expanded": open,
      })}
      {open && (
        <div className="popconfirm-popover" role="alertdialog" aria-label={message}>
          <div className="popconfirm-text">{message}</div>
          <div className="popconfirm-actions">
            <button
              type="button"
              className="btn btn-sm"
              onClick={() => setOpen(false)}
            >
              {cancelLabel}
            </button>
            <button
              type="button"
              className={`btn btn-sm ${danger ? "btn-danger" : "btn-primary"}`}
              onClick={() => void handleConfirm()}
            >
              {confirmLabel}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
