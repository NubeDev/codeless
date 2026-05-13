# Scope

Build a tiny in-memory key-value HTTP store in Go.

## In scope
- One main.go file
- Single in-process map[string]string protected by a mutex
- Listens on :8080
- Three HTTP routes that operate on the shared map

## Out of scope
- Persistence
- Auth
- Tests
