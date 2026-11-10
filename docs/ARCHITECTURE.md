# Architecture

Script Portal follows **clean architecture** with **DDD** building blocks. The target picture is
[`ScriptPortal.png`](ScriptPortal.png). Layers are separate crates, so a dependency that breaks a layer
rule must first be added to a `Cargo.toml`, and `sql-portal/integration-tests/tests/architecture.rs` rejects it.

## Crates

```
common/                       datasources/                   sql-portal/
  config-loader   (empty)       sql-executor  ◄──┐             domain ──────────► sql-executor
  template-render               sqlite-dialect ──┤             application ─────► domain
  prometheus-metrics            postgres-dialect ┤             adapters ────────► domain, sql-executor, *-dialect, tls-config
  tls-config                    clickhouse-dialect┘            api ─────────────► application, domain, template-render
                                                               deployment ──────► all of the above, prometheus-metrics
                                                               testing (dev) ───► domain
                                                               integration-tests ► everything (tests only)
```

| crate | role | may depend on (workspace crates) |
|---|---|---|
| `sql-portal-domain` | model and ports; no web, database or identity-provider crates | `sql-executor` (placeholder scanning) |
| `sql-portal-application` | use cases | `domain` |
| `sql-portal-adapters` | port implementations | `domain`, the datasource crates, `tls-config` |
| `sql-portal-api` | the only crate that knows axum/tower: routes, handlers, extractors, middleware, views, `serve`; the `Readiness` port | `domain`, `application`, `template-render` |
| `sql-portal` (deployment) | composition root + `script-portal` binary; health checks and observability; no axum | all layers, `prometheus-metrics` |
| `sql-portal-testing` | in-memory fakes of the ports | `domain`; only as a dev-dependency |
| `sql-portal-integration-tests` | integration and end-to-end tests | everything |
| `sql-executor` | dialect contract (`Dialect`, `Cell`, `Statement`), no I/O | — |
| `sqlite-` / `postgres-` / `clickhouse-dialect` | one dialect each, no I/O | `sql-executor` |
| `prometheus-metrics` | Prometheus recorder with caller-supplied buckets, install, render | — |
| `tls-config` | `TlsOptions`, rustls client config, reqwest helper (`reqwest` feature) | — |
| `template-render` | compiled minijinja template sets (autoescaped `.html`), `content_version` for cache busting | — |
| `config-loader` | shared library (not extracted yet) | — |

Only `sql-portal` and the integration tests depend on `sql-portal-api`; no other production crate depends on
axum or tower. `common/*` crates depend on nothing in `sql-portal/` or `datasources/`.

## Domain (`sql-portal/domain`)

Bounded contexts, one folder each. Ports live in the context they belong to and are re-exported at the
crate root (`sql_portal_domain::QueryExecutor`).

| context | concepts | ports |
|---|---|---|
| `common` | `Value`, `DomainError` | — |
| `access` | `Permission`, `PermissionSet`, `User`, `Principal`, `AccessPolicy`, `LocalGrants` | `CredentialAuthenticator`, `OidcProvider`, `GithubLogin`, `GrantSource`, `ScriptAccessRule` |
| `query` | `QueryRequest`, `QueryResult`, `QueryError`, paging | `QueryExecutor` |
| `scripts` | `ScriptDefinition` (aggregate), `ParamDefinition`, `ChartDefinition`, `sql_template`, `ConfigScriptCatalog` (in-memory catalog) | `ScriptCatalog` |
| `telemetry` | `NoopTelemetry` | `Telemetry` |

## Application (`sql-portal/application`)

| module | use case |
|---|---|
| `auth` | `AuthService`: the master `enabled` switch, LDAP login, OIDC/GitHub flows (state check), principal resolution |
| `scripts` | `ScriptService`: list / view / run with permission checks, parameter binding, charts, telemetry |

## Adapters (`sql-portal/adapters`)

| module | implements |
|---|---|
| `sql` | `QueryExecutor` for SQLite/PostgreSQL (sqlx) and ClickHouse (HTTP); `connect_database` picks one by URL scheme; migrations |
| `auth` | `LdapAuthenticator`, `OidcAdapter`, `GithubAdapter` |
| `grants` | `RemoteGrants` (HTTP grants service) and `CachedGrants` (TTL cache) |
| `config` | YAML loading, validation, mapping to `RuntimeConfig` |

## API (`sql-portal/api`)

Everything HTTP, on axum: the router with its layers (sessions, security headers, request ids, tracing,
`track_metrics`), handlers, extractors, HTML views and templates, and `serve`. It declares the `http_*` metrics
(`describe_metrics`) and defines the `Readiness` port behind `/readyz`, which deployment implements.

## Deployment (`sql-portal/deployment`)

The only crate that knows concrete adapters. Each module configures one part of the application from the
`RuntimeConfig` and returns fully wired services:

| module | builds from config |
|---|---|
| `database` | the `QueryExecutor`, with migrations and init files applied |
| `access` | the `GrantSource` (local policy or external service) |
| `auth` | the `AuthService` and its providers |
| `scripts` | the `ScriptService` with catalog and access rule |
| `health` | the `HealthProbe` trait, `DatabaseProbe`/`OidcProbe`, and `HealthService` (implements `api::Readiness`) |
| `observability` | tracing, the Prometheus recorder (`prometheus-metrics`), metric declarations, `MetricsTelemetry` |
| `http` | the router from `sql-portal-api` |

`lib.rs` assembles them in `wire()`; `build_app_with(…, Overrides)` lets tests (or another crate) replace
single adapters. `main.rs` is the CLI only: `serve` (through `sql_portal_api::serve`), `check-config`, `healthcheck`.

## Request flow (script page)

```
GET /scripts/x?_run=1&p=v
  → WebPrincipal extractor: session → User → AuthService.principal (GrantSource) | redirect /login
  → ScriptService.view: catalog lookup, permission check (404 / 403)
  → ScriptService.run: bind_params (validation → 422) → QueryExecutor.execute (timeout, max_rows) → charts
  → views (escaped HTML; chart specs as JSON) → security headers → metrics middleware
```

## Tests

Tests are written at the lowest layer that can express the behaviour:

1. **Domain unit tests**: pure, in each context.
2. **Application unit tests**: use cases over the fakes in `sql-portal-testing`.
3. **Adapter unit tests**: SQL against in-memory SQLite, config mapping, LDAP escaping, OIDC claims.
4. **In-process integration tests** (`integration-tests/tests/*.rs`): the real router from the demo config,
   with fake identity providers. No Docker.
5. **Architecture tests** (`architecture.rs`): the crate dependency rules above.
6. **Container tests** (`--features docker`), with testcontainers:
   - `postgres.rs`, `clickhouse.rs`: the adapters against real servers. One container per test binary,
     shared through a lease and removed when the last test releases it. ClickHouse tests get a database each.
   - `portal_image.rs`: the shipped image, one container per test. Checks HTTP endpoints, metrics, security
     headers, the built-in `healthcheck` and `check-config` commands, a startup failure on invalid config,
     and the image talking to PostgreSQL over a Docker network.

   Containers are removed when their owner is dropped, also when a test panics. The details are in
   `integration-tests/src/containers.rs`.

## Extension points

- **Another database**: a dialect crate in `datasources/` implementing `sql_executor::Dialect`, plus a
  `QueryExecutor` adapter in `adapters/src/sql/` selected in `connect_database`. Add a container test.
- **Another identity provider**: a port next to `CredentialAuthenticator`/`OidcProvider` in `domain/access`,
  an adapter in `adapters/src/auth/`, wiring in `deployment/src/auth/`.
- **Persistent sessions**: replace `MemoryStore` in `api/src/router.rs` with a `tower-sessions` store.
- **More chart kinds / form widgets**: extend `ChartKind`/`ParamKind` in the domain, then config mapping and views.
