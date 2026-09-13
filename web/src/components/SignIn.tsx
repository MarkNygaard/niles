import { LogIn } from "lucide-react";
import { Button } from "@/components/ui/button";

export interface SignInProps {
  /** What Niles said when the last attempt was refused, if it was. */
  error?: string;
}

/**
 * The whole page, when nobody is signed in.
 *
 * A full-page screen rather than a banner over the dashboard, because
 * there is nothing behind it to look at: every request the page would
 * make is refused. Showing an empty house with a sign-in prompt on top
 * would be describing rooms it cannot see.
 */
export function SignIn({ error }: SignInProps) {
  // A plain link, not fetch: the browser has to *navigate* to GitHub,
  // and the binding cookie has to be set on a response it actually
  // follows. `next` brings you back where you were.
  const next = encodeURIComponent(
    window.location.pathname + window.location.search,
  );

  return (
    <main className="flex min-h-dvh flex-col items-center justify-center gap-6 px-6 py-12">
      <div className="flex flex-col items-center gap-3 text-center">
        <HouseMark />
        <h1 className="font-heading text-2xl font-semibold">Niles</h1>
        <p className="text-muted-foreground max-w-sm text-sm">
          The lights in the house, and how they behave. Sign in to reach them.
        </p>
      </div>

      <Button
        render={<a href={`/auth/github/start?next=${next}`} />}
        size="lg"
        className="gap-2"
      >
        <LogIn /> Sign in with GitHub
      </Button>

      {error && (
        <p className="text-destructive max-w-sm text-center text-sm">{error}</p>
      )}

      <p className="text-muted-foreground/80 max-w-sm text-center text-xs">
        A GitHub account on its own is not enough — the address it has
        verified has to be one Niles was told about.
      </p>
    </main>
  );
}

/** The same mark as the Home Screen icon, so the page is recognisably it. */
function HouseMark() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      className="size-14 text-foreground"
      aria-hidden
    >
      <path d="M10 12V8.964" />
      <path d="M14 12V8.964" />
      <path d="M15 12a1 1 0 0 1 1 1v2a2 2 0 0 1-2 2h-4a2 2 0 0 1-2-2v-2a1 1 0 0 1 1-1z" />
      <path d="M8.5 21H5a2 2 0 0 1-2-2v-9a2 2 0 0 1 .709-1.528l7-6a2 2 0 0 1 2.582 0l7 6A2 2 0 0 1 21 10v9a2 2 0 0 1-2 2h-5a2 2 0 0 1-2-2v-2" />
    </svg>
  );
}
