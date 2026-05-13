# Routing for stage 1

Use the standard library `net/http`, no third-party routers.

Wire `/kv/get` and `/kv/set` to two handler functions. Use a single
`sync.RWMutex` to protect the underlying map. Initialise the map
empty on startup.

The server should call `log.Printf("listening on %s", addr)` before
ListenAndServe so the log proves the server actually came up.
