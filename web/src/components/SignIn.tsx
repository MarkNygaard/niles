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
        {/* Set in real type because this page has a font to do it
            with — the launch image does not, which is why the name is
            not on that. `font-wordmark` is the same face the header
            inside the app uses, so signing in does not hand you a
            different Niles. */}
        <h1 className="font-wordmark text-5xl font-medium tracking-wide">
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
 * A vector trace, which is why the path data is long and why it is two
 * paths rather than one: the jacket is the first shape and the shirt,
 * collar and bow tie are the second, with the bow tie's wings, its knot
 * and the three buttons as holes in it. Filled by winding number, which
 * is what the trace assumes — no `fill-rule` here.
 *
 * Green here, where white is the rule everywhere else: the accent is
 * only ever wrong when it competes with amber for meaning, and this is
 * the one screen with no light on it to report.
 *
 * `web/public/butler.svg` is the same geometry as a standalone file,
 * and the icons and launch images are generated from it. Change one and
 * the other has to move too.
 */
function ButlerMark() {
  return (
    <svg
      viewBox="0 0 1254 1254"
      className="text-mark size-28 fill-current"
      aria-hidden
    >
      <g transform="translate(0,1254) scale(0.1,-0.1)">
        <path
          d="M4030 11306 c0 -211 4 -366 11 -398 15 -72 49 -139 153 -297 104 -157 126
          -203 136 -285 21 -169 -55 -331 -247 -527 l-74 -76 16 -64 c9 -35 39 -138 66
          -229 108 -361 162 -538 184 -610 23 -73 201 -666 271 -905 20 -66 44 -147 54
          -180 11 -33 53 -177 95 -320 42 -143 85 -287 95 -320 10 -33 32 -107 49 -165
          17 -58 55 -188 84 -290 30 -102 73 -250 96 -330 24 -80 55 -185 71 -235 15
          -49 47 -155 70 -235 23 -80 63 -219 90 -310 27 -91 62 -212 79 -270 287
          -1004 861 -2883 876 -2868 10 10 214 597 363 1043 137 412 289 887 472 1475
          78 250 162 520 187 600 25 80 53 172 63 205 31 99 88 288 155 505 56 180 147
          481 391 1290 64 213 82 269 268 875 52 171 131 432 176 580 45 149 115 377
          156 508 41 130 74 245 74 255 0 10 -29 46 -65 80 -80 78 -194 247 -220 327
          -25 76 -25 191 -1 262 10 29 65 124 122 210 58 87 117 183 132 213 l27 55 3
          388 3 388 -138 -104 c-336 -252 -561 -390 -1006 -613 -361 -180 -406 -208
          -520 -314 -102 -95 -184 -206 -249 -336 -72 -145 -69 -143 -331 -144 -255 0
          -270 6 -317 120 -36 86 -145 244 -223 324 -109 110 -197 165 -502 318 -530
          266 -696 369 -1171 731 l-24 19 0 -346z m1202 -1026 c255 -128 473 -241 485
          -252 42 -39 44 -59 41 -396 -3 -308 -4 -322 -24 -349 -15 -20 -148 -91 -490
          -262 -383 -191 -478 -235 -520 -239 -58 -5 -93 10 -139 58 l-30 31 -3 774 -2
          775 36 40 c20 22 50 44 68 50 72 22 99 11 578 -230z m2668 218 c25 -13 55
          -38 67 -57 l23 -34 0 -749 c0 -828 3 -792 -65 -844 -30 -22 -50 -29 -99 -31
          l-61 -4 -475 238 -475 238 -22 45 c-22 43 -23 53 -23 347 0 321 3 347 48 386
          21 19 908 464 952 477 45 14 85 10 130 -12z m-1383 -594 c58 -43 64 -69 61
          -281 l-3 -193 -38 -37 -37 -38 -236 0 -236 0 -34 37 -34 38 0 204 0 204 27
          33 c47 56 66 59 295 57 191 -3 209 -5 235 -24z m-186 -1904 c220 -41 374
          -256 340 -471 -23 -142 -108 -257 -238 -321 -53 -26 -80 -32 -147 -36 -181
          -9 -330 77 -408 236 -31 63 -33 72 -33 177 0 106 2 114 34 180 58 117 154
          195 284 230 64 17 98 18 168 5z m62 -1581 c273 -91 368 -422 189 -657 -73
          -95 -219 -158 -350 -150 -110 7 -180 37 -258 110 -88 83 -125 160 -131 277
          -6 105 12 174 66 258 42 64 66 88 127 126 96 59 243 74 357 36z m29 -1507
          c311 -153 311 -581 1 -734 -64 -31 -74 -33 -173 -33 -93 0 -111 3 -163 27
          -80 38 -160 117 -200 197 -30 62 -32 72 -32 176 0 98 3 116 27 167 48 102
          135 179 253 221 37 13 72 17 136 14 73 -3 97 -8 151 -35z"
        />
        <path
          d="M3334 10664 c-253 -266 -684 -722 -863 -914 -90 -96 -311 -332 -490 -523
          -179 -192 -328 -353 -329 -358 -6 -15 1 -22 351 -352 181 -170 325 -313 320
          -318 -4 -4 -181 -97 -393 -207 -527 -273 -793 -412 -804 -422 -4 -4 1 -21 12
          -37 11 -15 58 -89 105 -163 440 -707 1374 -2146 1584 -2440 22 -30 86 -122
          143 -205 100 -144 428 -590 535 -725 171 -217 220 -278 319 -400 127 -155
          440 -521 590 -689 216 -243 553 -615 717 -791 69 -74 159 -171 199 -215 40
          -44 240 -254 443 -468 l369 -387 367 359 c1233 1211 1906 1935 2544 2741 59
          74 120 151 135 170 153 192 476 637 722 995 389 567 1497 2242 1488 2250 -2
          2 -79 42 -173 90 -192 99 -610 316 -850 441 -88 46 -168 89 -178 97 -17 12 1
          32 225 242 133 126 289 274 345 329 l102 99 -72 80 c-73 82 -329 360 -622
          677 -561 607 -712 771 -900 978 -115 127 -213 231 -216 231 -3 1 -22 -54 -41
          -121 -83 -285 -273 -944 -334 -1155 -144 -500 -364 -1233 -558 -1863 -271
          -877 -395 -1273 -513 -1640 -47 -146 -101 -314 -119 -375 -347 -1106 -907
          -2758 -1164 -3435 -45 -118 -91 -241 -103 -272 -22 -61 -31 -70 -40 -40 -3 9
          -48 141 -101 292 -149 425 -351 1036 -521 1575 -55 173 -172 561 -395 1310
          -34 116 -118 392 -185 615 -68 223 -140 461 -160 530 -20 69 -67 224 -105
          345 -37 121 -105 342 -150 490 -45 149 -113 371 -150 495 -38 124 -81 268
          -95 320 -15 52 -46 158 -70 235 -43 138 -100 333 -216 735 -33 113 -84 288
          -114 390 -70 239 -160 564 -225 810 -29 107 -63 233 -76 280 -14 47 -45 162
          -70 255 -24 94 -47 183 -52 199 -8 28 -10 27 -168 -140z"
        />
      </g>
    </svg>
  );
}
