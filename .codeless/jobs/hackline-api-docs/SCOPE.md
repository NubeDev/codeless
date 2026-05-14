# Scope — hackline API documentation

Generate complete, accurate API documentation for the hackline
gateway REST API. The source of truth is the existing router at
`crates/hackline-gateway/src/api/router.rs` and the handler files
under `crates/hackline-gateway/src/api/`. The existing
`DOCS/REST-API.md` is a summary — this job produces the full
specification and examples.

## What success looks like

After all stages run cleanly, the repo contains:

- `docs/openapi.yaml` — OpenAPI 3.1 spec covering every endpoint
  in the gateway. Includes request/response schemas, auth
  requirements, status codes, and examples.
- `docs/api-examples.sh` — runnable curl commands for every
  endpoint. Each command has a comment explaining what it does.
  Uses `$BASE_URL` and `$TOKEN` variables so the user sets them
  once.
- `docs/API.md` — human-readable API reference. One section per
  resource (health, claim, devices, tunnels, users, audit, events).
  Each endpoint gets a table of parameters, request body, response
  body, status codes, and auth notes.

Each stage commits its own files to the job's branch with a
descriptive message. No file outside `docs/` changes.

## Endpoints to document (from the router)

### Health
- `GET /v1/health` — no auth required

### Claim (first-boot)
- `GET /v1/claim/status` — no auth required
- `POST /v1/claim` — no auth required; atomic first-owner setup

### Devices
- `GET /v1/devices` — list all devices
- `POST /v1/devices` — register a new device
- `GET /v1/devices/:id` — get one device
- `PATCH /v1/devices/:id` — update mutable fields (label, customer_id)
- `DELETE /v1/devices/:id` — remove a device
- `GET /v1/devices/:id/info` — device runtime info
- `GET /v1/devices/:id/health` — device health check

### Tunnels
- `GET /v1/tunnels` — list tunnels
- `POST /v1/tunnels` — create a tunnel
- `DELETE /v1/tunnels/:id` — remove a tunnel

### Users
- `GET /v1/users` — list users
- `POST /v1/users` — create a user
- `DELETE /v1/users/:id` — remove a user
- `POST /v1/users/:id/tokens` — mint a bearer token

### Audit
- `GET /v1/audit?cursor=&limit=` — cursor-based pagination

### Events (SSE)
- `GET /v1/events` — all device events
- `GET /v1/devices/:id/events` — per-device events

## Auth model

`Authorization: Bearer <token>` on every endpoint except
`GET /v1/health`, `GET /v1/claim/status`, and `POST /v1/claim`.
Token is minted via `POST /v1/users/:id/tokens`.

## Out of scope

- Generating code from the OpenAPI spec (SDKs, client libs).
- Hosting the docs (no doc server, no GitHub Pages setup).
- Modifying any Rust source code or handler logic.
- Adding OpenAPI annotations to the Rust code itself.
- Any change outside `docs/`.

## Constraints

- File encoding: UTF-8, LF line endings.
- OpenAPI version: 3.1.0 (YAML format).
- Curl examples must be copy-pasteable with only `$BASE_URL` and
  `$TOKEN` set.
- The API.md must be readable without any external renderer.
- All work happens in the job's worktree branch; never on `main`.
