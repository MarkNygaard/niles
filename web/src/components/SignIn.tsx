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
    <main className="flex min-h-dvh flex-col items-center justify-center gap-6 px-5 py-12">
      {/* Wide enough for the tagline to be one line, which is the width
          the whole page is then set to. */}
      <div className="flex w-full max-w-lg flex-col items-center gap-4 text-center">
        <ButlerMark />
        {/* Set in a serif because Niles is a butler, and in real type
            because this page has a font to do it with — the launch
            image does not, which is why the name is not on that. */}
        <h1
          className="text-5xl font-medium tracking-wide"
          style={{ fontFamily: "ui-serif, Georgia, 'Times New Roman', serif" }}
        >
          Niles
        </h1>
        {/* One line at every width, which is the whole point of an
            acronym: broken across two it stops being one. The size is
            what gives, not the line — it tracks the viewport down to a
            phone and stops growing once there is room to spare. */}
        <p className="text-muted-foreground/90 text-[clamp(0.5rem,2.35vw,0.8125rem)] tracking-[0.12em] whitespace-nowrap uppercase">
          Neural Intelligence, Lightweight Edge System
        </p>
        <p className="text-muted-foreground mt-2 max-w-sm text-balance text-sm">
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

/**
 * The same mark as the Home Screen icon, so the page is recognisably it.
 *
 * Green here, where white is the rule everywhere else: the accent is
 * only ever wrong when it competes with amber for meaning, and this is
 * the one screen with no light on it to report.
 *
 * The shirt is the drawn shape and the jacket is whatever is behind it;
 * the tie and the buttons are cut back out, which is what puts the tie
 * inside the collar rather than floating above it. `fill-rule` does the
 * cutting, so it is one path rather than a shape and three patches.
 */
function ButlerMark() {
  return (
    <svg viewBox="0 0 24 24" className="text-mark size-28 fill-current" aria-hidden>
      <path
        fillRule="evenodd"
        d="M5.8 3 H18.2 L12 20.5 Z
           M7.8 4.2 V7.6 L12 5.9 Z
           M16.2 4.2 V7.6 L12 5.9 Z
           M12.85 10.6 a0.85 0.85 0 1 1 -1.7 0 a0.85 0.85 0 1 1 1.7 0 Z
           M12.85 13.6 a0.85 0.85 0 1 1 -1.7 0 a0.85 0.85 0 1 1 1.7 0 Z
           M12.85 16.6 a0.85 0.85 0 1 1 -1.7 0 a0.85 0.85 0 1 1 1.7 0 Z"
      />
    </svg>
  );
}
