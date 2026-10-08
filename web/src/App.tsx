import { useEffect } from "react";
import { useQuery } from "@tanstack/react-query";
import { ChevronLeft } from "lucide-react";
import { ConfigPanel } from "@/components/ConfigPanel";
import { GroceriesPage } from "@/components/GroceriesPage";
import { MePage } from "@/components/MePage";
import { MyProfile } from "@/components/MyProfile";
import { RoomDashboard } from "@/components/RoomDashboard";
import { SignIn } from "@/components/SignIn";
import { TabBar } from "@/components/TabBar";
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api";
import { useRoute } from "@/lib/route";
import { useTheme } from "@/lib/theme";
import type { Theme } from "@/lib/theme";
import { cn } from "@/lib/utils";

interface Screen {
  /** The route this screen is, which an unknown one is not. */
  at: string;
  title: string;
  /** Where the back button goes, for a page inside a tab. */
  parent?: string;
  body: React.ReactNode;
}

/**
 * The house, and the few other places the tab bar leads.
 */
export function App() {
  const route = useRoute();
  // Held here, not on the Me page that shows the switch: following the
  // system theme as it changes has to keep happening on every page.
  const [theme, setTheme] = useTheme();
  // Not refetched on focus like everything else: signing out in another
  // tab should not yank this one to a sign-in screen mid-press. The
  // 401s would say so anyway, and on the next load.
  const auth = useQuery({
    queryKey: ["auth"],
    queryFn: api.authStatus,
    refetchOnWindowFocus: false,
    staleTime: Infinity,
  });

  // A new page starts at its top. `#root` is what scrolls, not the
  // document — see globals.css.
  useEffect(() => {
    document.getElementById("root")?.scrollTo(0, 0);
  }, [route]);

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

  const email = auth.data?.signed_in_as ?? undefined;
  const avatarUrl = auth.data?.avatar_url ?? undefined;
  const screen = screenFor(route, email, avatarUrl, theme, setTheme);

  return (
    // The bottom padding on a phone is the tab bar's height, so the last
    // card can scroll clear of it.
    <main className="mx-auto flex w-full max-w-5xl flex-col gap-4 px-4 pt-[calc(env(safe-area-inset-top)+1rem)] pb-[calc(env(safe-area-inset-bottom)+5rem)] sm:px-6 sm:pt-[calc(env(safe-area-inset-top)+1.5rem)] sm:pb-[calc(env(safe-area-inset-bottom)+1.5rem)]">
      <header className="flex items-center gap-2">
        {screen.parent && (
          <a
            href={screen.parent}
            aria-label="Back"
            className={cn(
              "text-muted-foreground hover:text-foreground -ml-2 flex size-9 shrink-0 items-center justify-center rounded-full transition-colors",
              "focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none",
            )}
          >
            <ChevronLeft className="size-5" />
          </a>
        )}
        {/* Large and plain, the way a phone app titles a screen: it
            says where you are rather than offering somewhere to go.

            The name is the exception, and is set as the wordmark — the
            same face, weight and tracking as the sign-in screen, at the
            size a header bar can carry. "Settings" is a screen title,
            not the name, so it stays in the heading face: a serif there
            would be the brand claiming to be a destination. */}
        {screen.at === "/" ? (
          <h1 className="font-wordmark flex-1 truncate text-2xl font-medium tracking-wide">
            Niles
          </h1>
        ) : (
          <h1 className="font-heading flex-1 truncate text-2xl font-semibold tracking-tight">
            {screen.title}
          </h1>
        )}
        <TabBar route={screen.at} email={email} avatarUrl={avatarUrl} />
      </header>

      {screen.body}
    </main>
  );
}

function screenFor(
  route: string,
  email: string | undefined,
  avatarUrl: string | undefined,
  theme: Theme,
  setTheme: (theme: Theme) => void,
): Screen {
  switch (route) {
    case "/groceries":
      return { at: route, title: "Groceries", body: <GroceriesPage /> };
    case "/me":
      return {
        at: route,
        title: "Me",
        body: <MePage email={email} avatarUrl={avatarUrl} theme={theme} onTheme={setTheme} />,
      };
    case "/me/profile":
      return { at: route, title: "My profile", parent: "#/me", body: <MyProfile /> };
    case "/me/settings":
      return { at: route, title: "Settings", parent: "#/me", body: <ConfigPanel /> };
    // Anything unknown is the house: an old bookmark or a mistyped hash
    // should land somewhere useful rather than on a blank page.
    default:
      return { at: "/", title: "Niles", body: <RoomDashboard /> };
  }
}
