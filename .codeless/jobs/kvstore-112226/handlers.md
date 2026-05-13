# Handler shape for stage 2

`GET /kv/get?key=<name>`: returns the value for `<name>` as plain text
with status 200, or empty body with status 404 if absent.

`POST /kv/set?key=<name>&value=<val>`: stores `<val>` under `<name>`,
returns "ok" with status 200.

`GET /kv/list`: returns one `key=value` per line in key-sorted order,
status 200. Empty body when the store is empty.

All handlers must Set Content-Type: text/plain; charset=utf-8.
Non-matching HTTP methods return 405 with body "method not allowed".
