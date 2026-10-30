# Docker Compose

[`compose.yaml`](https://github.com/MarkNygaard/niles/blob/main/deploy/compose/compose.yaml) runs Niles and its Postgres database as two
containers on one machine. The MQTT broker, Zigbee2MQTT and Piper run
wherever they run today, and Niles connects to them.

## What you need

- **A Linux machine with an x86-64 CPU**, running Docker Engine and the
  Compose plugin. The image is built for `linux/amd64` only, so a
  Raspberry Pi or another ARM board can't run it. Docker Desktop on a Mac
  or a Windows PC hides the address each connection comes from, and Niles
  tells the satellites apart by their addresses, so use Linux for a house
  with satellites.
- **An MQTT broker** that Zigbee2MQTT publishes to, with a username and a
  password for Niles.
- **An ElevenLabs API key** for hearing and **a Cerebras API key** for
  thinking. Cerebras has a free tier.
- **Piper's HTTP server** to speak the replies. Niles can start without
  it. See [Piper](#piper) below.

## Start it

```sh
git clone https://github.com/MarkNygaard/niles
cd niles/deploy/compose
cp .env.example .env
```

1. In `.env`, fill in every value. The file says how to make the database
   password and the secrets key.
2. In `niles.toml`, set `host` to your MQTT broker's IP address or host
   name, as the Niles container reaches it.
3. Start the stack:

   ```sh
   docker compose up -d
   docker compose logs -f niles
   ```

4. Open `http://<the machine's address>:8080`.

The first start creates the database's tables, a session key and an API
key. `docker compose ps` shows both containers as healthy once Niles
answers on `/healthz`.

## The files

| File | What it holds |
|---|---|
| `compose.yaml` | The two services, the ports, the healthchecks and the database volume. |
| `.env` | The passwords and keys, made from `.env.example`. Keep it out of git. |
| `niles.toml` | The broker's address, and the names of the variables in `.env` that Niles reads before the app is up. |

Niles needs the broker's login and the two API keys at its first start: it
connects to the broker and builds its hearing and thinking clients before
the app is up. If it can't reach the broker, it stops a minute after
starting and Compose starts it again, so `docker compose logs niles` is the
place to look when it keeps restarting.

Everything else is set in the app and kept in Postgres: the lighting
curve, Sonos, accounts, add-ons and their keys. A key that comes from
`.env` shows in the app as "from the environment" and is changed in
`.env`.

The Compose file sets `NILES_SUPERVISOR: docker` beside
`restart: unless-stopped`, which is what lets Settings offer
**Restart now**. See [Restarting](/docs/self-hosting#restarting).

## Upgrading

`latest` is the newest release, and pulling it upgrades Niles:

```sh
docker compose pull
docker compose up -d
```

To upgrade only when you choose to, set `NILES_IMAGE` in `.env` to a
version, such as `ghcr.io/marknygaard/niles:0.4.0`, and change it for each
upgrade. The releases are listed on
[GitHub](https://github.com/MarkNygaard/niles/releases). Niles creates the
tables it needs by itself, so there is no migration step.

## Backing up

Everything Niles keeps is in the database, on the `niles_db` volume. To
back it up:

```sh
docker compose exec -T db pg_dump -U niles niles > niles-backup.sql
```

To restore it into a fresh stack, start only the database, load the file,
then start Niles:

```sh
docker compose up -d db
docker compose exec -T db psql -U niles niles < niles-backup.sql
docker compose up -d
```

Keep `NILES_SECRET_KEY` with the backup. The keys typed into the app are
sealed with it, and a database restored under another key has to have
each of them typed in again.

## Piper

Niles speaks through Piper's HTTP server (`python3 -m piper.http_server`
from the [Piper](https://github.com/OHF-Voice/piper1-gpl) project, on port
5000). It is not `wyoming-piper`, which speaks a different protocol. Run it
on a machine Niles can reach, then enter its base address, such as
`http://192.168.1.11:5000`, under **Settings → Voice & AI → Speaking**.
Leave the path off: Niles finds the current server's `/synthesize` and the
older servers' `/` by itself.

## Satellites

Each satellite streams to port 10300 on the machine running Compose, so
put that machine's IP address in the satellite's settings. Niles calls each
satellite back on its port 10301 to ring timers and speak announcements,
which needs no port opened on the Niles side.
