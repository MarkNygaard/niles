import { useEffect } from "react";
import { useQuery } from "@tanstack/react-query";
import { ChevronLeft } from "lucide-react";
import { ChatPage } from "@/components/ChatPage";
import { ConfigPanel } from "@/components/ConfigPanel";
import { GroceriesPage } from "@/components/GroceriesPage";
import { MediaPage, somethingPlays } from "@/components/MediaPage";
import { AvatarMenu } from "@/components/AvatarMenu";
import { MyProfile } from "@/components/MyProfile";
import { RoomDashboard } from "@/components/RoomDashboard";
import { SignIn } from "@/components/SignIn";
import { TabBar } from "@/components/TabBar";
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api";
import { menuLayout } from "@/lib/menu";
import { useRoute } from "@/lib/route";
import { useTheme } from "@/lib/theme";
import { cn } from "@/lib/utils";

interface Screen {
  /** The route this screen is, which an unknown one is not. */
  at: string;
  title: string;
  /** The page sizes its own bottom: the chat pins its composer there,
      and the page's padding under it would only be a gap. */
  ownsBottom?: boolean;
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
  const refused = Boolean(auth.data?.enabled && !auth.data.signed_in_as);
  // The menu's arrangement lives in the config, which the dashboard
  // asks for anyway: one query, shared through its key.
  const config = useQuery({
    queryKey: ["config"],
    queryFn: api.getConfig,
    enabled: !auth.isLoading && !refused,
  });
  // What plays, for whether the Media entry is in the menu. The same
  // queries the dashboard and the Media page use, so one fetch serves.
  const music = useQuery({
    queryKey: ["music"],
    queryFn: api.music,
    enabled: !auth.isLoading && !refused,
    retry: false,
    refetchInterval: 15_000,
  });
  const tv = useQuery({
    queryKey: ["tv"],
    queryFn: api.tv,
    enabled: !auth.isLoading && !refused,
    retry: false,
    refetchInterval: 60_000,
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
  if (refused) {
    return <SignIn error={refusal ?? undefined} />;
  }

  const email = auth.data?.signed_in_as ?? undefined;
  const avatarUrl = auth.data?.avatar_url ?? undefined;
  const screen = screenFor(route);
  const layout = config.isPending
    ? undefined
    : menuLayout(config.data?.effective, somethingPlays(music.data, tv.data?.status?.on));

  return (
    // The bottom padding on a phone is the tab bar's height, so the last
    // card can scroll clear of it.
    <main
      className={cn(
        "mx-auto flex w-full max-w-5xl flex-col gap-2 px-4 pt-[calc(env(safe-area-inset-top)+0.25rem)] sm:gap-4 sm:px-6 sm:pt-[calc(env(safe-area-inset-top)+1.5rem)]",
        !screen.ownsBottom &&
          "pb-[calc(env(safe-area-inset-bottom)+3.5rem)] sm:pb-[calc(env(safe-area-inset-bottom)+1.5rem)]",
      )}
    >
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
        {/* Nothing after Home until the arrangement is known: an entry
            somebody hid should not flash up on every load. */}
        <TabBar route={screen.at} menu={layout?.main ?? []} />
        <AvatarMenu
          email={email}
          avatarUrl={avatarUrl}
          items={layout?.avatar ?? []}
          theme={theme}
          onTheme={setTheme}
        />
      </header>

      {screen.body}
    </main>
  );
}

function screenFor(route: string): Screen {
  switch (route) {
    case "/groceries":
      return { at: route, title: "Groceries", body: <GroceriesPage /> };
    case "/media":
      return { at: route, title: "Media", body: <MediaPage /> };
    case "/chat":
      return { at: route, title: "Chat", ownsBottom: true, body: <ChatPage /> };
    // "#/me" was the Me page before the avatar menu took its place; an
    // old bookmark to it lands on the profile.
    case "/me":
    case "/me/profile":
      return { at: "/me/profile", title: "My profile", body: <MyProfile /> };
    case "/me/settings":
      return { at: route, title: "Settings", body: <ConfigPanel /> };
    // Anything unknown is the house: an old bookmark or a mistyped hash
    // should land somewhere useful rather than on a blank page.
    default:
      return { at: "/", title: "Niles", body: <RoomDashboard /> };
  }
}
