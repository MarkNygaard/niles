# Kubernetes Deployment

This directory contains a [Kustomize](https://kustomize.io/) base and overlays for deploying Niles to Kubernetes.

## Prerequisites

- A Kubernetes cluster (v1.28+)
- `kubectl` configured to talk to your cluster
- `kustomize` CLI installed (ships with `kubectl kustomize` or standalone)
- Container image `ghcr.io/marknygaard/niles` available in your cluster (or build/push it first)

## Quick Start

### 1. Create required out-of-band resources

The manifests reference a Secret and (for production) a PVC that you must create yourself:

```bash
# Production namespace
kubectl create namespace niles

# Secrets (never commit these to git)
kubectl create secret generic niles-secrets \
  --namespace niles \
  --from-literal=mqtt-username='your-mqtt-user' \
  --from-literal=mqtt-password='your-mqtt-pass' \
  --from-literal=groq-api-key='your-groq-key' \
  --from-literal=openai-api-key='your-openai-key'

# Production data PVC (adjust storageClassName and size to your cluster)
cat <<EOF | kubectl apply -f -
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: niles-data
  namespace: niles
spec:
  accessModes:
    - ReadWriteOnce
  resources:
    requests:
      storage: 5Gi
  storageClassName: standard
EOF
```

For dev:

```bash
kubectl create namespace niles-dev
kubectl create secret generic niles-secrets \
  --namespace niles-dev \
  --from-literal=mqtt-username='your-mqtt-user' \
  --from-literal=mqtt-password='your-mqtt-pass' \
  --from-literal=groq-api-key='your-groq-key' \
  --from-literal=openai-api-key='your-openai-key'
```

### 2. Preview changes

```bash
# Dev
kustomize build deploy/kustomize/overlays/dev | kubectl diff -f -

# Production
kustomize build deploy/kustomize/overlays/production | kubectl diff -f -
```

### 3. Apply

```bash
# Dev
kubectl apply -k deploy/kustomize/overlays/dev

# Production
kubectl apply -k deploy/kustomize/overlays/production
```

### 4. Verify

```bash
# Dev
kubectl get pods -n niles-dev
kubectl logs -n niles-dev -l app.kubernetes.io/name=niles

# Production
kubectl get pods -n niles
kubectl logs -n niles -l app.kubernetes.io/name=niles
```

## Directory Layout

```
deploy/kustomize/
├── base/
│   ├── kustomization.yaml      # Base resources
│   ├── deployment.yaml         # Niles Deployment (placeholder image, emptyDir data)
│   ├── service.yaml            # ClusterIP Service (8080 + 10300)
│   └── configmap.yaml          # Placeholder ConfigMap
├── overlays/
│   ├── production/
│   │   ├── kustomization.yaml  # Production patches + configMapGenerator
│   │   ├── niles.toml          # Aarhus home config, da_DK locale
│   │   └── deployment-prod.yaml  # Bumps resources + replaces emptyDir with PVC
│   └── dev/
│       ├── kustomization.yaml  # Dev patches + configMapGenerator
│       └── niles.toml          # Dev-cluster config
```

## Customising

Create your own overlay by copying `overlays/dev/` and adjusting:

- `niles.toml` — coordinates, locale, MQTT host, lighting schedule
- `kustomization.yaml` — image tag, namespace, labels
- Add patches for resource limits, node selectors, tolerations, etc.

## Bumping Versions

Pin the image tag in each overlay's `kustomization.yaml`:

```yaml
images:
  - name: niles
    newName: ghcr.io/marknygaard/niles
    newTag: v0.2.0   # <-- bump here
```

Then preview and apply:

```bash
kustomize build deploy/kustomize/overlays/production | kubectl diff -f -
kubectl apply -k deploy/kustomize/overlays/production
```

## What's NOT Here

The following are intentionally out of scope for v1. You may add them in your own overlays or future PRs:

- **GitOps / ArgoCD / FluxCD** — apply manually for now
- **Helm** — Kustomize is the chosen tool
- **Multi-cluster / per-region overlays**
- **CI-driven image builds** — build and push the image separately
- **Monitoring / observability sidecars** — no Prometheus or log shippers
- **Network policies**
- **HPA / VPA** — Niles runs single-replica
- **Secrets management** — raw `kubectl create secret` is used
- **Ingress / TLS** — wire via your existing ingress controller out-of-band
- **Mosquitto, SearXNG, Piper manifests** — deploy independently

## Ports

| Name | Port | Purpose |
|------|------|---------|
| `http-api` | 8080 | HTTP API (`/healthz`, `/devices`, etc.) |
| `wyoming` | 10300 | Wyoming voice-protocol server |

## Health Checks

Both liveness and readiness probes hit `GET /healthz` on port 8080.
