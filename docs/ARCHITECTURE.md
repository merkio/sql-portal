# Architecture

Script Portal follows **clean architecture** with **DDD** building blocks. Source dependencies point inwards only.

```
        ┌──────────────────────────────────────────────────────┐
        │ presentation   axum routes, extractors, HTML views    │
        │ infrastructure config, sqlx, ldap3, openidconnect,    │
        │                metrics/tracing adapters                │
        │ bootstrap      composition root: wire() + router()    │
        └──────────────┬───────────────────────────────────────┘
                       ▼
        ┌──────────────────────────────┐
        │ application   use cases       │  AuthService, ScriptService, HealthService
        └──────────────┬───────────────┘
                       ▼
        ┌──────────────────────────────┐
        │ domain        model + ports   │  no web / DB / LDAP / OIDC dependencies
        └──────────────────────────────┘
```

## Layers

### `domain/`
Split into bounded contexts. A context only depends on the ones above it in this table (see `domain/mod.rs`):

| Context | Concepts (DDD role) | Where |
|---|---|---|
| `shared` | Common kernel: `Value` (value object), `DomainError` | `shared/` |
| `access` | `Permission`, `PermissionSet`: value objects with wildcard matching | `access/` |
| `identity` | `User` (asserted by a provider), `Principal` (user + resolved permissions), `AccessPolicy` (domain service: users/groups → roles → permissions), `AuthError` | `identity/` |
| `query` | `QueryRequest`, `QueryResult`, `QueryError` | `query/` |
| `scripts` | `ScriptDefinition` (aggregate; invariants enforced in `validate()`), `ParamDefinition`, `ChartDefinition`, `sql_template` (`:name` placeholders, read-only check), `ScriptError`, `FieldError` | `scripts/` |
| `ports` | One file per seam: `CredentialAuthenticator` and `OidcProvider` (`auth`), `QueryExecutor` (`query`), `ScriptCatalog` (`catalog`), `ScriptAccessRule` (`script_access`), `HealthProbe` (`health`), `Telemetry` (`telemetry`) | `ports/` |

Callers keep using the flat names (`domain::Principal`, `domain::ports::QueryExecutor`).

### `application/`
Use cases orchestrate domain objects through ports, with no knowledge of HTTP or SQL drivers.
- `AuthService`: master `enabled` switch, LDAP login, OIDC begin/complete (state check), principal resolution.
- `ScriptService`: list/view/run with permission checks, parameter binding, chart preparation, telemetry.

### `infrastructure/`
Adapters implementing ports, organized by concern:
- `health/`: `HealthService` (aggregates `HealthProbe`), `DatabaseProbe`, `OidcProbe`
- `sql/`: `SqlExecutor` (sqlx `Any`), database connections, migrations
- `config/`: YAML config loading, mapping, runtime configuration
- `auth/`: `LdapAuthenticator` (ldap3), `OidcAdapter` (openidconnect), `GithubAdapter`
- `catalog/`: `ConfigScriptCatalog`
- `grants/`: `RemoteGrants`, `CachedGrants`, `LocalGrants`
- `observability/`: `MetricsTelemetry` (metrics + Prometheus), tracing

### `presentation/`
axum router, extractors (`WebPrincipal` redirects to `/login`, `ApiPrincipal` answers 401), handlers, HTML views with
escaping, middleware (route-template metrics, security headers).

### `bootstrap.rs`
The only place that knows concrete adapters. Builds services from configuration and returns the router.

## SOLID in practice

- **S**ingle responsibility: parameter parsing (`ParamDefinition`), placeholder scanning (`sql_template`), placeholder rewriting (`named_params`),
  chart extraction (`ChartDefinition::build`) and HTML rendering (`views`) are separate, individually tested units.
- **O**pen/closed: new databases, directories or identity providers are new adapters; use cases do not change.
- **L**iskov: every port has an in-memory fake (`src/testing.rs`) that is substitutable for the real adapter in tests.
- **I**nterface segregation: small ports (`HealthProbe` has three methods; `Telemetry` two).
- **D**ependency inversion: application code depends on traits in `domain::ports`; `bootstrap` injects implementations.

## TDD approach

Tests live next to the code they specify and drive each layer from the inside out:

1. **Domain unit tests** (pure, microsecond): permissions, policy resolution, parameter parsing, placeholder scanning, charts, script invariants.
2. **Application tests** with fakes: login/state checks, permission filtering, bounded metric labels, error classification, readiness semantics.
3. **Adapter tests**: SQL against in-memory SQLite (types, truncation, injection attempts), config loading/validation, LDAP filter escaping and entry mapping, OIDC claim mapping.
4. **HTTP integration tests** (`tests/`): the real router with fake LDAP/OIDC and seeded SQLite — login, CSRF, session rotation, roles via groups, forms, API, health, metrics, headers.
5. **Composition-root tests** (`tests/bootstrap.rs`): the shipped configs build a working app; bad configs fail fast.

Add a feature by writing the failing test at the lowest sensible layer, then the code, then an HTTP test if it changes behaviour.

## Request flow (script page)

```
GET /scripts/x?_run=1&p=v
  → WebPrincipal extractor: session → User → AuthService.principal (policy.resolve) | redirect /login
  → ScriptService.view: catalog lookup, permission check (404 / 403)
  → normalise checkboxes, ScriptService.run:
        bind_params (validation → 422)  →  QueryExecutor.execute (timeout, max_rows)  →  charts
  → views::script_body (escaped HTML; chart specs as JSON blocks)  →  security headers  →  metrics middleware
```

## Extension points

- **Another database**: write a `dialect-<name>` crate implementing `sql_dialect::Dialect` (placeholder
  rewriting, parameter binding, result parsing, no I/O), then a `QueryExecutor` adapter in
  `infrastructure/sql/` that does the I/O and selects the URL scheme in `connect_database`. The ClickHouse
  and PostgreSQL crates are worked examples.
- **Persistent sessions**: replace `MemoryStore` in `presentation/router.rs` with a `tower-sessions` store (sqlx, Redis…).
- **More auth providers** (e.g. SAML, static tokens): implement a port next to `CredentialAuthenticator`/`OidcProvider` and add it to `AuthService`.
- **More chart kinds / form widgets**: extend `ChartKind`/`ParamKind` in the domain, then the config mapping and `views`.
