import { useEffect, useState } from "react";

/**
 * Temporary. What the phone actually thinks the screen is.
 *
 * Five attempts at the gap under the sheet have been reasoned from
 * descriptions and every one was wrong, so this prints the numbers
 * instead. It comes out in the same change as the fix it leads to.
 */
export function ViewportProbe({ of }: { of: HTMLElement | null }) {
  const [lines, setLines] = useState<string[]>([]);

  useEffect(() => {
    const read = () => {
      const probe = document.createElement("div");
      probe.style.cssText =
        "position:fixed;top:0;left:0;height:100dvh;width:env(safe-area-inset-bottom);visibility:hidden";
      document.body.appendChild(probe);
      const dvh = probe.getBoundingClientRect().height;
      const insetBottom = probe.getBoundingClientRect().width;
      probe.remove();

      const body = document.body.getBoundingClientRect();
      const bodyStyle = getComputedStyle(document.body);
      const box = of?.getBoundingClientRect();

      setLines([
        `inner ${window.innerHeight} client ${document.documentElement.clientHeight} dvh ${Math.round(dvh)}`,
        `vv ${Math.round(window.visualViewport?.height ?? 0)}+${Math.round(window.visualViewport?.offsetTop ?? 0)} inset-b ${Math.round(insetBottom)}`,
        `body ${Math.round(body.top)}..${Math.round(body.bottom)} pos ${bodyStyle.position} ovf ${bodyStyle.overflowY}`,
        `body pad ${bodyStyle.paddingTop}/${bodyStyle.paddingBottom} h ${bodyStyle.height}`,
        box
          ? `sheet ${Math.round(box.top)}..${Math.round(box.bottom)} h ${Math.round(box.height)}`
          : "sheet —",
        `scrollY ${Math.round(window.scrollY)} bodyScroll ${document.body.scrollTop}`,
      ]);
    };
    read();
    // Again after a frame: what it is at open and what it settles to
    // are the two numbers that matter, and they may differ.
    const timer = setTimeout(read, 400);
    return () => clearTimeout(timer);
  }, [of]);

  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-0 z-50 bg-black/70 px-2 py-1 font-mono text-[10px] leading-tight text-white">
      {lines.map((line) => (
        <div key={line}>{line}</div>
      ))}
    </div>
  );
}
