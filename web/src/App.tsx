import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { ChevronLeft } from "lucide-react";
import { AccountMenu } from "@/components/AccountMenu";
import { ConfigPanel } from "@/components/ConfigPanel";
import { RoomDashboard } from "@/components/RoomDashboard";
import { SignIn } from "@/components/SignIn";
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";

type View = "home" | "settings";

/**
 * The house, with everything about *you* behind the avatar.
 *
 * No tab bar. The house is the page you came for; how it looks and who
 * you are are not a second destination of equal weight, and a phone app
 * would not give them half the top bar.
 */
export function App() {
  const [view, setView] = useState<View>("home");
  // Not refetched on focus like everything else: signing out in another
  // tab should not yank this one to a sign-in screen mid-press. The
  // 401s would say so anyway, and on the next load.
  const auth = useQuery({
    queryKey: ["auth"],
    queryFn: api.authStatus,
    refetchOnWindowFocus: false,
    staleTime: Infinity,
  });

  // Nothing is worth drawing before we know whether it will be refused.
  if (auth.isLoading) {
    return (
      <main className="mx-auto flex w-full max-w-5xl flex-col gap-4 px-4 py-6">
        <Skeleton className="h-8 w-40" />
        <Skeleton className="h-64 w-full" />
      </main>
    );
  }

  // A refused sign-in comes back on the URL rather than in a body,
  // because the browser followed a redirect to get here.
  const refusal = new URLSearchParams(window.location.search).get("sign_in_error");
  if (auth.data?.enabled && !auth.data.signed_in_as) {
    return <SignIn error={refusal ?? undefined} />;
  }

  const settings = view === "settings";

  return (
    <main className="mx-auto flex w-full max-w-5xl flex-col gap-4 px-4 py-4 sm:px-6 sm:py-6">
      <header className="flex items-center gap-2">
        {settings && (
          <button
            type="button"
            aria-label="Back to the house"
            onClick={() => setView("home")}
            className={cn(
              "text-muted-foreground hover:text-foreground -ml-2 flex size-9 shrink-0 items-center justify-center rounded-full transition-colors",
              "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
            )}
          >
            <ChevronLeft className="size-5" />
          </button>
        )}
        {/* Large and plain, the way a phone app titles a screen: it says
            where you are rather than offering somewhere to go. */}
        <h1 className="font-heading flex-1 truncate text-2xl font-semibold tracking-tight">
          {settings ? "Settings" : "Niles"}
        </h1>
        <AccountMenu
          email={auth.data?.signed_in_as ?? undefined}
          onOpenSettings={() => setView("settings")}
        />
      </header>

      {settings ? <ConfigPanel /> : <RoomDashboard />}
    </main>
  );
}
