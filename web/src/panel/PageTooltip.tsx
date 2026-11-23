// Tooltips drawn inside the overlay page instead of Chromium's native ones.
//
// A native `title` tooltip is a separate popup window: it sticks out of the panel over the game,
// is not excluded from screen capture, and — lying over the panel's edge — swallows the press that
// should start a resize (seen with the ✕ button's tooltip over the bottom-right corner). So on hover
// the `title` attribute moves to `data-tip` (the native tooltip never appears) and the text is shown
// here, kept inside the window. Components keep using plain `title`.

import { useEffect, useLayoutEffect, useRef, useState } from "react";

const DELAY_MS = 500;
const GAP = 6;
const MARGIN = 4;

interface Tip {
  text: string;
  anchor: DOMRect;
}

export function PageTooltip() {
  const [tip, setTip] = useState<Tip | null>(null);
  const boxRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let current: HTMLElement | null = null;
    let timer: number | undefined;

    const hide = () => {
      window.clearTimeout(timer);
      current = null;
      setTip(null);
    };

    const onOver = (e: PointerEvent) => {
      const el = (e.target as Element | null)?.closest?.<HTMLElement>("[title], [data-tip]") ?? null;
      if (el === current) return;
      hide();
      current = el;
      if (!el) return;
      // React may set `title` again after a re-render: the latest value wins.
      const title = el.getAttribute("title");
      if (title !== null) {
        el.dataset.tip = title;
        el.removeAttribute("title");
      }
      const text = el.dataset.tip;
      if (!text) return;
      timer = window.setTimeout(() => {
        if (current === el && el.isConnected) setTip({ text, anchor: el.getBoundingClientRect() });
      }, DELAY_MS);
    };

    const onOut = (e: PointerEvent) => {
      if (!e.relatedTarget) hide(); // left the window
    };

    document.addEventListener("pointerover", onOver);
    document.addEventListener("pointerout", onOut);
    document.addEventListener("pointerdown", hide, true);
    document.addEventListener("wheel", hide, { capture: true, passive: true });
    window.addEventListener("blur", hide);
    return () => {
      window.clearTimeout(timer);
      document.removeEventListener("pointerover", onOver);
      document.removeEventListener("pointerout", onOut);
      document.removeEventListener("pointerdown", hide, true);
      document.removeEventListener("wheel", hide, { capture: true });
      window.removeEventListener("blur", hide);
    };
  }, []);

  // Below the element if it fits, else above, else over it; always inside the window.
  useLayoutEffect(() => {
    const box = boxRef.current;
    if (!tip || !box) return;
    const { width, height } = box.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const a = tip.anchor;
    let top = a.bottom + GAP;
    if (top + height > vh - MARGIN) top = a.top - GAP - height;
    if (top < MARGIN) top = a.top + (a.height - height) / 2;
    top = Math.max(MARGIN, Math.min(top, vh - height - MARGIN));
    let left = a.left + a.width / 2 - width / 2;
    left = Math.max(MARGIN, Math.min(left, vw - width - MARGIN));
    box.style.transform = `translate(${Math.round(left)}px, ${Math.round(top)}px)`;
    box.style.visibility = "visible";
  }, [tip]);

  if (!tip) return null;
  return (
    <div ref={boxRef} className="page-tooltip" role="tooltip">
      {tip.text}
    </div>
  );
}
