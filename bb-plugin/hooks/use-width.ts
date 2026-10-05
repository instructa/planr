import { useEffect, useState, type RefObject } from "react";

/** Width of an element, tracked with a ResizeObserver; 0 before the first layout. */
export function useElementWidth(ref: RefObject<HTMLElement | null>): number {
  const [width, setWidth] = useState(0);
  useEffect(() => {
    const el = ref.current;
    if (el === null || typeof ResizeObserver === "undefined") return;
    const update = () => setWidth(el.clientWidth);
    update();
    const observer = new ResizeObserver(update);
    observer.observe(el);
    return () => observer.disconnect();
  }, [ref]);
  return width;
}

/** Bare keys only, outside text fields and open overlays (bb's Tasks rule). */
export function isBoardKey(event: KeyboardEvent): boolean {
  if (event.defaultPrevented || event.metaKey || event.ctrlKey || event.altKey) return false;
  const target = event.target;
  if (target instanceof HTMLElement) {
    if (
      target instanceof HTMLInputElement ||
      target instanceof HTMLTextAreaElement ||
      target instanceof HTMLSelectElement
    ) {
      return false;
    }
    if (target.isContentEditable) return false;
  }
  return document.querySelector('[role="dialog"], [role="menu"], [role="listbox"]') === null;
}
