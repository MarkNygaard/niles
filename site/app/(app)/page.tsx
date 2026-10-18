import type { Metadata } from "next"
import Link from "next/link"

import { siteConfig } from "@/lib/config"
import { SiteFooter } from "@/components/site-footer"

const tagline = "A home assistant you talk to, run on your own cluster"

export const metadata: Metadata = {
  title: { absolute: `${siteConfig.name}. ${tagline}.` },
  description: siteConfig.description,
  alternates: { canonical: "/" },
}

const turn = [
  {
    title: "Heard in the room",
    body: `The satellite hears "${siteConfig.name}" itself, with a wake word model trained for that one word. Nothing streams until it is said, and the room's Sonos turns down while ${siteConfig.name} listens.`,
    who: "The satellite",
  },
  {
    title: "Fixed phrasings first",
    body: "Lights, blinds, scenes, music, the TV, timers, heating, the weather and the clock are matched by regex and carried out with no language model. The target is about half a second end to end.",
    who: "Tier 0",
  },
  {
    title: "A fast model for the rest",
    body: "Anything else goes to a fast model (Cerebras by default) with only the tools that request is likely to need. You can say it your own way, and it takes a second or so.",
    who: "Tier 1",
  },
  {
    title: "The assistant for bigger jobs",
    body: `Making a rule, writing a travel plan or a podcast, or reading a photo goes to Claude Code or ChatGPT on the household's own subscription. It reaches the same tools through ${siteConfig.name}'s own MCP server.`,
    who: "The assistant",
  },
  {
    title: "Answered where you asked",
    body: "Piper speaks the reply on the satellite that heard you. A reply that ends in a question keeps the microphone open, so you can answer without the wake word.",
    who: "The satellite",
  },
]

const features = [
  {
    title: "Its own satellites",
    body: `A reSpeaker XVF3800 with a XIAO ESP32-S3, running ${siteConfig.name}'s own ESP-IDF firmware. The XMOS chip cancels echo, so it hears you over music. Every satellite is identical and keeps no room state.`,
    href: "/docs/setting-up/satellites",
  },
  {
    title: "The lighting curve",
    body: "A brightness and a colour temperature for every minute of the day, applied once a minute to the lights that are on. It never switches a light. One you set yourself is left alone until it goes off and on.",
    href: "/docs/using/lighting",
  },
  {
    title: "Saved for you",
    body: "Ask for a packing list, a trip, a training plan for a race or a podcast episode. The assistant writes it, the last 50 versions are kept, and you can share it with someone in the house to view or to edit.",
    href: "/docs/using/saved",
  },
  {
    title: "Approvals",
    body: `${siteConfig.name} never sends a mail by itself. A mail, or a tool set to Ask first, waits as a card in the app until you press Allow once, and what runs is exactly what the card showed.`,
    href: "/docs/using/approvals",
  },
  {
    title: "Automations",
    body: "\"When the garden door has been open for ten minutes, tell me.\" Say it, or draw it on the Automations page. Rules are stored in Postgres, and a rule that is waiting survives a restart.",
    href: "/docs/using/automations",
  },
  {
    title: "Add-ons from GitHub",
    body: "Anything not built in can be an add-on: a WebAssembly component from a GitHub release. It reaches only the hosts and devices the household approved, and never the database, conversations or other add-ons' passwords.",
    href: "/add-ons",
  },
  {
    title: "One house, several people",
    body: `${siteConfig.name} knows who is speaking from an enrolled voice. Your calendar, mail, workouts and chats are yours. The lights, the scenes and the curve belong to the house.`,
    href: "/docs/using/people",
  },
  {
    title: "Chat in the app",
    body: `The same ${siteConfig.name}, typed, on a phone or a computer. Attach a photo or a PDF and the assistant reads it. Spoken turns from your voice show up in your chat too.`,
    href: "/docs/using/chat",
  },
  {
    title: "No telemetry",
    body: `${siteConfig.name} sends nothing about you or your use of it to its authors. The security page lists every outside service it does use, and what goes to each.`,
    href: "/docs/using/security",
  },
]

const builtIn = [
  ["Zigbee2MQTT", "lights, plugs, sensors, switches and blinds, named room/device"],
  ["WLED", "LED strips, as a second MQTT source"],
  ["Sonos", "music and radio in each room, and Spotify through it"],
  ["LG webOS", "TVs over their local websocket, woken by Wake-on-LAN"],
  ["tado° and UniFi", "the heating, and who is home"],
  ["Microsoft, Google, iCloud", "each person's calendars and mail"],
  ["Strava", "each person's workouts, from their own Strava app"],
  ["Linear", "issues and projects, through Linear's own MCP server"],
  ["SearXNG", "the house's own web search, for the assistant"],
]

/**
 * WebSite plus SoftwareApplication: the first tells a crawler what this site
 * is and anchors the docs' `isPartOf`; the second answers "what is this, what
 * does it run on, what does it cost" without reading the prose.
 */
const jsonLd = {
  "@context": "https://schema.org",
  "@graph": [
    {
      "@type": "WebSite",
      "@id": `${siteConfig.url}/#website`,
      url: siteConfig.url,
      name: siteConfig.name,
      description: siteConfig.description,
      inLanguage: "en",
    },
    {
      "@type": "SoftwareApplication",
      "@id": `${siteConfig.url}/#software`,
      name: siteConfig.name,
      description: siteConfig.description,
      applicationCategory: "HomeApplication",
      operatingSystem: "Linux, Kubernetes",
      url: siteConfig.url,
      codeRepository: siteConfig.links.github,
      license: ["https://opensource.org/licenses/MIT", "https://www.apache.org/licenses/LICENSE-2.0"],
      isAccessibleForFree: true,
      offers: { "@type": "Offer", price: "0", priceCurrency: "USD" },
    },
  ],
}

// From deploy/README.md: the quick start, applied to an overlay of your own.
// The resource names are the manifests' own, which follow the binary's name.
const bin = siteConfig.binary
const install = `git clone ${siteConfig.links.github}
cd ${siteConfig.links.github.split("/").pop()}
cp -r deploy/kustomize/overlays/dev deploy/kustomize/overlays/home
# In overlays/home, set ${bin}.toml, and the namespace in kustomization.yaml

kubectl create namespace ${bin}
kubectl create secret generic ${bin}-secrets \\
  --namespace ${bin} \\
  --from-literal=mqtt-username='your-mqtt-user' \\
  --from-literal=mqtt-password='your-mqtt-pass'

kustomize build deploy/kustomize/overlays/home | kubectl diff -f -
kubectl apply -k deploy/kustomize/overlays/home`

export default function HomePage() {
  return (
    <>
      <script
        type="application/ld+json"
        // Trusted, locally constructed values -- no user input reaches this.
        dangerouslySetInnerHTML={{ __html: JSON.stringify(jsonLd) }}
      />
      <section className="mx-auto w-full max-w-6xl px-6 pt-20 pb-16">
        <p className="text-sm font-medium text-accent-orange">
          Self-hosted · open source · {siteConfig.license}
        </p>
        <h1 className="mt-4 max-w-3xl text-4xl font-semibold tracking-tight text-balance sm:text-5xl">
          {tagline}
        </h1>
        <p className="mt-5 max-w-2xl text-lg leading-relaxed text-muted-foreground">
          Say &quot;{siteConfig.name}&quot; to the satellite in the room and ask
          for what you want. Common commands are carried out without a language
          model. The rest go to a fast model with tools, and the bigger jobs to
          Claude or ChatGPT on your own subscription. What {siteConfig.name}{" "}
          keeps is in your own Postgres.
        </p>

        <div className="mt-8 flex flex-wrap items-center gap-3">
          <Link
            href="/docs"
            className="inline-flex h-9 items-center rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground transition-opacity hover:opacity-90"
          >
            Read the docs
          </Link>
          <a
            href={siteConfig.links.github}
            target="_blank"
            rel="noreferrer"
            className="inline-flex h-9 items-center rounded-md border border-border px-4 text-sm font-medium transition-colors hover:bg-accent"
          >
            View source
          </a>
        </div>
      </section>

      <section className="mx-auto w-full max-w-6xl px-6 pb-16">
        <h2 className="text-2xl font-semibold tracking-tight">
          How a spoken command is answered
        </h2>
        <p className="mt-3 max-w-2xl leading-relaxed text-muted-foreground">
          Each step runs only when the one before it could not answer. A light
          switched by voice never waits on a model, a plugin or a cloud call
          it does not need.
        </p>

        <ol className="mt-8 grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {turn.map((step, i) => (
            <li
              key={step.title}
              className="rounded-lg border border-border bg-card p-5"
            >
              <span className="font-mono text-xs text-muted-foreground">
                {String(i + 1).padStart(2, "0")}
              </span>
              <h3 className="mt-2 font-medium">{step.title}</h3>
              <p className="mt-1.5 text-sm leading-relaxed text-muted-foreground">
                {step.body}
              </p>
              <p className="mt-3 text-xs font-medium text-accent-orange">
                {step.who}
              </p>
            </li>
          ))}
        </ol>

        <p className="mt-6 max-w-2xl text-sm leading-relaxed text-muted-foreground">
          <Link
            href="/docs/using/talking"
            className="underline underline-offset-4 hover:text-foreground"
          >
            Talking to {siteConfig.name}
          </Link>{" "}
          lists what you can say, and{" "}
          <Link
            href="/docs/architecture/voice-loop"
            className="underline underline-offset-4 hover:text-foreground"
          >
            the voice loop
          </Link>{" "}
          follows a command through each tier and the prompt the model gets.
        </p>

        <p className="mt-3 max-w-2xl text-sm leading-relaxed text-muted-foreground">
          <Link
            href="/why"
            className="underline underline-offset-4 hover:text-foreground"
          >
            Why {siteConfig.name}
          </Link>{" "}
          says who it is for, who should stay on Home Assistant, and what
          leaves the house.
        </p>
      </section>

      <section className="mx-auto w-full max-w-6xl px-6 pb-16">
        <h2 className="text-2xl font-semibold tracking-tight">
          Someone in the house runs a cluster
        </h2>
        <p className="mt-3 max-w-2xl leading-relaxed text-muted-foreground">
          {siteConfig.name} is one container and a Postgres database on your
          own Kubernetes, beside an MQTT broker and Zigbee2MQTT. It reaches
          the database through{" "}
          <code className="rounded bg-muted px-1.5 py-0.5 text-sm">{siteConfig.envPrefix}DATABASE_URL</code>{" "}
          and seals the keys you type into the app with{" "}
          <code className="rounded bg-muted px-1.5 py-0.5 text-sm">{siteConfig.envPrefix}SECRET_KEY</code>.
        </p>
        <pre className="mt-6 max-w-3xl overflow-x-auto rounded-lg border border-border bg-card p-4 text-sm whitespace-pre">
          <code>{install}</code>
        </pre>
        <p className="mt-4 max-w-2xl text-sm leading-relaxed text-muted-foreground">
          The image is{" "}
          <code className="rounded bg-muted px-1.5 py-0.5">{siteConfig.image}</code>;
          pin its tag in the overlay. The overlay&apos;s {siteConfig.binary}.toml
          holds the coordinates, the locale, the MQTT host and the lighting
          schedule.{" "}
          <Link
            href="/docs/operations/deployment"
            className="underline underline-offset-4 hover:text-foreground"
          >
            Deployment
          </Link>{" "}
          covers signing in, the data volume and the ports. There is no Docker
          Compose file.
        </p>
      </section>

      <section className="mx-auto w-full max-w-6xl px-6 pb-16">
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {features.map((feature) => (
            <Link
              key={feature.title}
              href={feature.href}
              className="rounded-lg border border-border bg-card p-5 transition-colors hover:border-ring"
            >
              <h2 className="font-medium">{feature.title}</h2>
              <p className="mt-2 text-sm leading-relaxed text-muted-foreground">
                {feature.body}
              </p>
            </Link>
          ))}
        </div>
      </section>

      <section className="mx-auto w-full max-w-6xl px-6 pb-24">
        <h2 className="text-2xl font-semibold tracking-tight">Built in</h2>
        <p className="mt-2 max-w-2xl text-muted-foreground">
          Each of these is a crate in the repository, held to its tests.
          Anything else is an{" "}
          <Link
            href="/add-ons"
            className="underline underline-offset-4 hover:text-foreground"
          >
            add-on
          </Link>
          .
        </p>
        <dl className="mt-6 divide-y divide-border border-y border-border">
          {builtIn.map(([name, what]) => (
            <div key={name} className="flex flex-wrap gap-x-4 gap-y-1 py-3">
              <dt className="w-56 text-sm font-medium">{name}</dt>
              <dd className="text-sm text-muted-foreground">{what}</dd>
            </div>
          ))}
        </dl>
      </section>

      <SiteFooter />
    </>
  )
}
