import { useEffect, useState } from "react";
import { Puzzle } from "lucide-react";
import type { LucideIcon } from "lucide-react";

type Icons = Record<string, LucideIcon>;

let loading: Promise<Icons> | null = null;
let loaded: Icons | null = null;

/** Every Lucide icon, fetched once, in a chunk of its own: only a house
    with a plugin page in its menu ever downloads it. */
function loadIcons(): Promise<Icons> {
  loading ??= import("lucide-react/dist/esm/icons/index.js").then((module) => {
    loaded = module as Icons;
    return loaded;
  });
  return loading;
}

/** `arrow-up-right` as Lucide's component is named: `ArrowUpRight`. */
export function componentName(icon: string): string {
  return icon
    .split("-")
    .filter(Boolean)
    .map((part) => part[0].toUpperCase() + part.slice(1))
    .join("");
}

/**
 * The Lucide icon a plugin's menu entry names (`zap`), or a puzzle
 * piece while the icons load and for a name Lucide does not have.
 */
export function PluginIcon({ name, className }: { name: string; className?: string }) {
  const [icons, setIcons] = useState<Icons | null>(loaded);
  useEffect(() => {
    if (icons) return;
    let live = true;
    loadIcons()
      .then((all) => live && setIcons(all))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [icons]);
  const Icon = icons?.[componentName(name)] ?? Puzzle;
  return <Icon aria-hidden className={className} />;
}
