# Workflow — hackline API documentation

How the agent drives the three stages. Each stage is an independent
add-only change. A failure in stage 2 still leaves stage 1's OpenAPI
spec on the branch.

## Stage 1 — OpenAPI spec

Read every handler file under `crates/hackline-gateway/src/api/` to
extract request bodies, response shapes, query parameters, path
parameters, and status codes. Cross-reference with the types in
`crates/hackline-proto/src/` for shared structs.

Write `docs/openapi.yaml`:

- `openapi: "3.1.0"`, `info.title: "hackline gateway"`.
- One path entry per endpoint listed in SCOPE.md.
- Reusable `#/components/schemas` for Device, Tunnel, User, Event,
  AuditEntry, ClaimStatus, and error shapes.
- Security scheme: `bearerAuth` (HTTP bearer token).
- Mark health and claim endpoints as not requiring auth.
- Include `example` values on request/response schemas where the
  handler source makes the shape clear.

Commit as `hackline-api-docs stage 1: OpenAPI 3.1 spec`.
Verify: the file must be valid YAML and contain every path from
SCOPE.md.

## Stage 2 — curl examples

Write `docs/api-examples.sh`. Structure:

```bash
#!/usr/bin/env bash
# hackline API examples — set BASE_URL and TOKEN before running
BASE_URL="${BASE_URL:-http://127.0.0.1:7447}"
TOKEN="${TOKEN:-your-token-here}"
```

Then one section per resource group (health, claim, devices,
tunnels, users, audit, events). Each curl command:

- Uses `-H "Authorization: Bearer $TOKEN"` where auth is required.
- Uses `-H "Content-Type: application/json"` for POST/PATCH.
- Shows a realistic JSON body for create/update endpoints.
- Includes a comment line above explaining what the call does.
- Uses `jq .` for readability where the response is JSON.

Commit as `hackline-api-docs stage 2: curl examples`.
Verify: `bash -n docs/api-examples.sh` exits 0 (syntax check).

## Stage 3 — human-readable API reference

Write `docs/API.md`. Sections in order:

- **Overview** — two sentences on what the API is.
- **Authentication** — how bearer tokens work, which endpoints
  are public.
- **Health** — endpoint table.
- **Claim** — endpoint table + note on atomicity.
- **Devices** — endpoint table with request/response fields,
  mutable fields note on PATCH.
- **Tunnels** — endpoint table.
- **Users** — endpoint table + token minting.
- **Audit** — endpoint table + cursor pagination explanation.
- **Events (SSE)** — endpoint table + note on proxy buffering
  (flush_interval for Caddy).
- **Error responses** — common error shapes and status codes.

Each endpoint entry includes: method, path, auth required (yes/no),
request body fields (if any), response body fields, status codes.

Commit as `hackline-api-docs stage 3: API reference markdown`.
Verify: file exists and is non-empty.

## What counts as done

All three stages committed cleanly on the job branch. No edits
outside `docs/`. The OpenAPI spec covers every endpoint. The curl
script passes `bash -n` syntax check. The markdown is self-contained.

## What to avoid

- Do not modify any Rust source files.
- Do not add OpenAPI code generation tooling or build steps.
- Do not invent endpoints that don't exist in the router.
- Do not push the branch. The user reviews the worktree before
  any push happens.
