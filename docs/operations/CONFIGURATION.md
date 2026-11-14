# Configuration (env)

| Var | Default | Notes |
|---|---|---|
| DATABASE_URL | — | required for persistence; absent → 503 on business routes |
| AUTH_MODE | jwt | `jwt` or `dev` (dev refused when ENVIRONMENT=production) |
| OIDC_ISSUER / OIDC_JWKS_URL / OIDC_AUDIENCE | — | required when AUTH_MODE=jwt |
| ENVIRONMENT | local | `production` hardening |
| BIND_ADDR | 0.0.0.0:8080 | |
| SERVICE_NAME / SERVICE_VERSION | iaop-agency-service / crate version | |
| ACTIVATION_MODE | fail_closed | legacy scaffold knob, unused by agency domain |
| OUTBOX_* | poll/batch/max_attempts | publisher tuning |
