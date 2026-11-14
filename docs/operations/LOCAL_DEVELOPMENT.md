# Local Development

Prereqs: Docker (postgres:16), Rust stable, vcvars64 env on Windows.

```powershell
# DB
docker run -d --name local-postgresql16 -e POSTGRES_PASSWORD=<pw> -p 5432:5432 postgres:16
# create agency_db via psql

# Run service (dev auth bypass — refused in production)
$env:DATABASE_URL="postgres://postgres:<pw>@127.0.0.1:5432/agency_db"
$env:AUTH_MODE="dev"; $env:ENVIRONMENT="local"; $env:BIND_ADDR="127.0.0.1:8080"
cargo run

# Real-DB tests
$env:DATABASE_URL=...; cargo test --test db_tests
```

JWT mode: set `AUTH_MODE=jwt` + `OIDC_ISSUER`, `OIDC_JWKS_URL`,
`OIDC_AUDIENCE` (see CONFIGURATION.md).
