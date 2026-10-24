# Kubernetes

This directory has a [Kustomize](https://kustomize.io/) base and two
overlays, `dev` and `production`, for running Niles on Kubernetes. For one
machine with Docker,
[`deploy/compose`](https://github.com/MarkNygaard/niles/tree/main/deploy/compose)
is simpler.

## What you need

- A Kubernetes cluster (1.28 or newer) with `linux/amd64` nodes. The image
  `ghcr.io/marknygaard/niles` is public and built for x86-64 only.
- `kubectl`. Kustomize is built in (`kubectl apply -k`).
- **A Postgres database** the pod can reach. Bring your own: a Postgres
  you already run, a managed one, or one from an operator such as
  CloudNativePG. Niles needs a database and a role that owns it, and no
  extensions. It creates its own tables on first use.
- **An MQTT broker** that Zigbee2MQTT publishes to, with a login for Niles.
- **An ElevenLabs API key** for hearing and **a Cerebras API key** for
  thinking.
- **Piper's HTTP server**, to speak the replies. Its base address, such as
  `http://piper:5000`, is set in the app, under Settings → Voice & AI. The
  current server (`/synthesize`) and older ones (`/`) both work. Niles
  starts without it.

## 1. The database

On your Postgres, as a superuser:

```sql
CREATE ROLE niles LOGIN PASSWORD 'choose-one';
CREATE DATABASE niles OWNER niles;
```

The connection string is then
`postgres://niles:choose-one@<host>:5432/niles`. Use letters and digits in
the password, or percent-encode it, because it sits inside a URL.

## 2. The Secret

Niles reads two values from outside on every start, and three more on the
first:

| Key in `niles-secrets` | Variable | What it is |
|---|---|---|
| `database-url` | `NILES_DATABASE_URL` | The Postgres connection string. Required. |
| `secret-key` | `NILES_SECRET_KEY` | 32 random bytes, base64. Seals every key typed into the app. Required. |
| `mqtt-username`, `mqtt-password` | `NILES_MQTT_USERNAME`, `NILES_MQTT_PASSWORD` | The broker's login. |
| `elevenlabs-api-key` | `ELEVENLABS_API_KEY` | Hearing. |
| `cerebras-api-key` | `CEREBRAS_API_KEY` | Thinking. |

Niles connects to the broker and builds its hearing and thinking clients
before the app is up, so those last three have to be in the Secret before
the first start. It also exits a minute after starting when it can't reach
the broker. Everything else is typed into the app and sealed in Postgres.

```sh
kubectl create namespace niles

kubectl create secret generic niles-secrets \
  --namespace niles \
  --from-literal=database-url='postgres://niles:choose-one@postgres.example:5432/niles' \
  --from-literal=secret-key="$(openssl rand -base64 32)" \
  --from-literal=mqtt-username='niles' \
  --from-literal=mqtt-password='...' \
  --from-literal=elevenlabs-api-key='...' \
  --from-literal=cerebras-api-key='...'
```

Keep a copy of the secret key with your database backups. Lose it or
change it, and every key stored in the app has to be typed in again.

The Deployment reads each key by name (`secretKeyRef`). If your cluster
mounts the whole Secret with `envFrom: secretRef` instead, name the keys
after the variables (`NILES_DATABASE_URL` and so on): Kubernetes drops keys
with hyphens from `envFrom` without an error.

## 3. An overlay

Copy the `dev` overlay and make it yours:

```sh
cp -r deploy/kustomize/overlays/dev deploy/kustomize/overlays/home
```

- In `kustomization.yaml`, set `namespace` and the image tag.
- In `niles.toml`, set `[mqtt] host` to your broker. The file's `*_env`
  lines name the variables from the Secret; they hold no values.
  Coordinates, the timezone and the lighting schedule can be set here or
  in the app.

## 4. Apply

```sh
kubectl kustomize deploy/kustomize/overlays/home | kubectl diff -f -
kubectl apply -k deploy/kustomize/overlays/home
```

Check that it came up:

```sh
kubectl get pods -n niles
kubectl logs -n niles -l app.kubernetes.io/name=niles
```

The log says `[secrets] ... loaded from the database` and
`[auth] made an API key and kept it` on a first start with a working
database and key.

## Reaching it

The Service is a `ClusterIP` with two ports:

| Name | Port | For |
|---|---|---|
| `http-api` | 8080 | The app and the API. `/healthz` answers the probes. |
| `wyoming` | 10300 | The satellites, which stream audio to it. |

- **The app.** Put it behind your ingress controller with TLS.
- **The satellites.** Expose 10300 on the house network with a
  `LoadBalancer` or `NodePort` Service that sets
  `externalTrafficPolicy: Local`. Niles knows which room a satellite is in
  by its IP address, and the default policy replaces that address with a
  node's. Niles also calls each satellite on its port 10301, which needs
  the pod to reach the satellites' network.

## Signing in

Sign-in is off, and the app open to anyone who reaches it, until a GitHub
app is set up and somebody is listed. The first person opens the app from
the house network, puts the GitHub OAuth app's client ID and secret in the
**Signing in** card under **Settings → People**, and adds their own GitHub
email address to the list below it.

To have the deployment own them instead, add `github-client-id` and
`github-client-secret` to the Secret and name the variables in
`niles.toml`:

```toml
[auth]
github_client_id_env = "NILES_GITHUB_CLIENT_ID"
github_client_secret_env = "NILES_GITHUB_CLIENT_SECRET"
```

The OAuth app's callback URL is `https://<your niles host>/auth/github/callback`.

With a database, Niles makes and keeps its own session key and API key.
The Deployment also reads two optional Secret keys, for a deployment that
sets them itself:

- `session-secret` signs the session cookie. Changing it signs everybody
  out.
- `api-token` is the API key, read only when `[auth] api_token_env =
  "NILES_API_TOKEN"` names it. Otherwise copy Niles's own from
  **Settings → API access**.

## Upgrading

Pin the image tag in the overlay's `kustomization.yaml`:

```yaml
images:
  - name: ghcr.io/marknygaard/niles
    newTag: "0.4.0"
```

Tags are `latest` (the newest release), each version (`0.4.0`) and
`sha-<commit>`. Change the tag, preview and apply. Niles creates the tables
it needs by itself, so there is no migration step.

## Backing up

Everything Niles keeps is in Postgres, so back up the database, for
example with `pg_dump`, and keep the secret key beside it. The pod's
`/data` is working space for Claude Code and pi, and an `emptyDir` is
enough.

## Layout

```
deploy/kustomize/
├── base/
│   ├── kustomization.yaml
│   ├── deployment.yaml     # Niles, its probes and the Secret's keys
│   ├── service.yaml        # ClusterIP: 8080 and 10300
│   └── configmap.yaml      # Placeholder niles.toml
└── overlays/
    ├── dev/                # namespace niles-dev, :latest
    └── production/         # namespace niles, a pinned version, resource limits
```

Each overlay replaces the ConfigMap with its own `niles.toml`. Both mount
an `emptyDir` at `/data`.

## Not here

- Postgres, Mosquitto, Zigbee2MQTT, Piper and SearXNG. Run them as you
  already do. SearXNG is added in the app, under Settings → Add-ons → Web
  search (SearXNG).
- Ingress and TLS, which belong to your ingress controller.
- Helm charts, and more than one replica. Niles runs as one pod.
