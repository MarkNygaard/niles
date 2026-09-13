import { useState } from "react";
import { Home, SlidersHorizontal } from "lucide-react";
import { ConfigPanel } from "@/components/ConfigPanel";
import { RoomDashboard } from "@/components/RoomDashboard";
import { cn } from "@/lib/utils";

type View = "home" | "settings";

/**
 * Two pages: the house, and how it behaves.
 *
 * The house comes first. Settings are read once a month; lights are
 * pressed every day, and the thing you open this on your phone to do
 * should not be behind a tab.
 */
export function App() {
  const [view, setView] = useState<View>("home");

  return (
    <main className="mx-auto flex w-full max-w-5xl flex-col gap-4 px-4 py-6 sm:px-6 sm:py-8">
      <header className="flex flex-wrap items-center justify-between gap-3">
        <h1 className="font-heading text-xl font-semibold">Niles</h1>
        <nav className="bg-muted/60 flex items-center gap-1 rounded-lg p-1">
          <NavButton
            current={view}
            value="home"
            icon={<Home aria-hidden className="size-4" />}
            onSelect={setView}
          >
            Home
          </NavButton>
          <NavButton
            current={view}
            value="settings"
            icon={<SlidersHorizontal aria-hidden className="size-4" />}
            onSelect={setView}
          >
            Settings
          </NavButton>
        </nav>
      </header>

      {view === "home" ? <RoomDashboard /> : <ConfigPanel />}
    </main>
  );
}

function NavButton({
  current,
  value,
  icon,
  children,
  onSelect,
}: {
  current: View;
  value: View;
  icon: React.ReactNode;
  children: React.ReactNode;
  onSelect: (view: View) => void;
}) {
  const active = current === value;
  return (
    <button
      type="button"
      aria-current={active ? "page" : undefined}
      onClick={() => onSelect(value)}
      className={cn(
        "focus-visible:ring-3 focus-visible:ring-ring/50 flex h-8 items-center gap-1.5 rounded-md px-3 text-sm font-medium transition-colors focus-visible:outline-none",
        active
          ? "bg-background text-foreground shadow-sm"
          : "text-muted-foreground hover:text-foreground",
      )}
    >
      {icon}
      {children}
    </button>
  );
}
