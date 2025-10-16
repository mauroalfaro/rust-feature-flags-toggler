# rust-feature-flags-toggler

Small feature flags service to toggle features on and off and support simple percentage rollouts and weighted variants. It’s meant to be easy to run locally and simple to integrate from other services.

## What it does
- Stores flags in SQLite
- Exposes a JSON API over HTTP (Axum)
- Evaluates flags for a given `user_id` with stable hashing
- Supports boolean flags, percentage rollouts, and weighted variants

## Local Setup
- Prereqs: recent Rust toolchain (`rustup`), SQLite available on the machine
- Env vars:
  - `DATABASE_URL` (default `sqlite://flags.db`)
  - `BIND` (default `0.0.0.0:8080`)

Run locally:
```
RUST_LOG=info DATABASE_URL=sqlite://flags.db cargo run
```

Smoke test:
```
curl http://localhost:8080/health

curl -X POST http://localhost:8080/flags \
  -H "content-type: application/json" \
  -d '{"key":"new-homepage","enabled":true,"variants":{"a":50,"b":50},"rollout":50}'

curl -X POST http://localhost:8080/evaluate \
  -H "content-type: application/json" \
  -d '{"key":"new-homepage","user_id":"123"}'
```

## Docker
You can also run it in a container for consistency.

Build:
```
docker build -t rust-feature-flags-toggler .
```

Run (with a volume for the database):
```
docker run --rm -p 8080:8080 \
  -e RUST_LOG=info \
  -e DATABASE_URL=sqlite:///data/flags.db \
  -v %CD%\data:/data \
  rust-feature-flags-toggler
```
On Linux/macOS, replace the volume path with `$(pwd)/data:/data`.

## API
- `GET /health` – health check
- `GET /flags` – list flags
- `GET /flags/:key` – get a flag by key
- `POST /flags` – create a flag
- `PATCH /flags/:key` – update a flag
- `DELETE /flags/:key` – delete a flag
- `POST /evaluate` – evaluate a flag with context

### Example Requests/Responses (JSON)

Create:
```
POST /flags
{
  "key": "new-homepage",
  "enabled": true,
  "variants": { "a": 50, "b": 50 },
  "rollout": 50
}
```
Response:
```
{
  "id": 1,
  "key": "new-homepage",
  "enabled": true,
  "variants": { "a": 50, "b": 50 },
  "rollout": 50,
  "updated_at": "2024-01-01T00:00:00Z"
}
```

List:
```
GET /flags
[
  {
    "id": 1,
    "key": "new-homepage",
    "enabled": true,
    "variants": { "a": 50, "b": 50 },
    "rollout": 50,
    "updated_at": "2024-01-01T00:00:00Z"
  }
]
```

Evaluate:
```
POST /evaluate
{
  "key": "new-homepage",
  "user_id": "123"
}
```
Response:
```
{ "key": "new-homepage", "matched": true, "variant": "a" }
```

## Notes
- Variant weights are integers and must sum to a positive number
- `rollout` is 0–100 and gates evaluation by `user_id`
- If no variants are set, the flag behaves as a boolean gate

## Docker Compose
You can also use docker-compose for quick local runs.

Compose file maps port 8080 and persists the SQLite DB under `./data`.

Up/Down:
```
docker compose up -d
# ... app runs on http://localhost:8080
curl http://localhost:8080/health

docker compose down
```

Override env in an `.env` file next to `docker-compose.yml` (optional):
```
RUST_LOG=info
DATABASE_URL=sqlite:///data/flags.db
BIND=0.0.0.0:8080
```
Then rerun `docker compose up -d`.
