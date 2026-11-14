# Security Model

- JWT validation via local `JwksTokenValidator` (issuer/JWKS URL/audience
  from `OIDC_*` env) — same semantics as `iaop-security` but NOT yet the
  shared crate (PARTIAL: pinned shared-crate migration pending).
- `DevBypassValidator` accepts a fixed admin principal; refused when
  `ENVIRONMENT=production`.
- `AuthPrincipal` extractor on every business route → 401 missing/invalid.
- Permissions (`agency:*`) enforced in application commands via
  `permissions::require` → 403. Fail-closed when services/db absent (503).
- Client-supplied identity headers are never trusted; principal comes
  only from validated JWT claims (sub/iss/scope/realm+audience-scoped
  resource_access roles).
- Raw tokens/secrets are not logged.
