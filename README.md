# Script Portal

A small web service that turns **SQL scripts defined in a YAML file** into authenticated web pages.
Each script gets a generated parameter form, a result table and optional charts. Users sign in
with **LDAP** and/or **OpenID Connect (OIDC)**; what they may run is controlled by a
role → permission model in the same config. Authentication can be switched off globally with a
single flag.

Built with Rust, `axum` and `tokio`; structured as clean architecture / DDD, developed test-first.

## Features

- **Config-driven scripts**: SQL (inline or from a file), typed parameters (`text`, `integer`,
  `number`, `boolean`, `date`, `select`), and charts (`line`, `bar`, `pie`, `doughnut`, `scatter`).
  Parameters are **bound**, never interpolated into SQL.
- **LDAP login** (search-then-bind with a service account, group lookup via `memberOf` or a group search).
- **OIDC login** (authorization code flow + PKCE, ID-token verification, group claim).
- **One flag disables every authorizer**: `auth.enabled: false`.
- **Permissions in config**: roles carry permissions (`*` and `prefix.*` wildcards); users and
  groups carry roles; each script lists the permissions it requires.
- **Pages**: `/` lists the scripts you may run; `/scripts/<name>` shows the form and results.
  JSON API under `/api/scripts`.
- **Operations**: `/healthz` (liveness), `/readyz` (readiness, checks the database),
  `/metrics` (Prometheus), `tracing` logs (text or JSON) with request ids.
- **Docker image** (distroless, non-root, built-in `HEALTHCHECK`) and release binaries.

## Project structure

One Cargo workspace (root `Cargo.toml`) in three groups:

```
common/          shared libraries, independent of the portal
  config-loader/     (placeholder, not used yet)
  template-render/   compiled minijinja template sets, content versions
  prometheus/        Prometheus recorder setup and rendering (prometheus-metrics)
  tls/               TLS options for rustls and reqwest clients (tls-config)
datasources/     SQL dialects, no I/O
  sql-executor/      the dialect contract (Dialect trait, Cell, Statement)
  sqlite/ postgres/ clickhouse/   one dialect each, built on sql-executor
sql-portal/      the application
  domain/            model and ports, no web/database/identity-provider crates
  application/       use cases: auth/, scripts/
  adapters/          port implementations: sql/, auth/, grants/, config/
  api/               everything axum: routes, handlers, views, middleware, server loop
  deployment/        composition root (one module per configured part, incl. health/ and
                     observability/) + main.rs
  testing/           in-memory fakes of the ports (dev-dependency only)
  integration-tests/ in-process, database and Docker-image tests
```

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the layer rules.

## Quick start

```bash
# demo: authentication disabled, in-memory SQLite with generated sales data
cargo run -- --config config/config.demo.yaml
# open http://localhost:8080
```

```bash
# or with Docker (the image defaults to the demo config)
docker build -t script-portal .
docker run --rm -p 8080:8080 script-portal

# portal + Prometheus
docker compose up --build
```

> ⚠️ The bundled **demo config disables authentication**. Never expose it beyond a trusted machine.

Useful commands:

```bash
script-portal --config my.yaml check-config    # validate configuration and exit
script-portal --config my.yaml serve           # run (default)
script-portal healthcheck --addr 127.0.0.1:8080 --path /readyz
```

`SCRIPT_PORTAL_CONFIG` (config path) and `SCRIPT_PORTAL_BIND` (listen address) may be set as environment variables.

## Configuration

See [`config/config.example.yaml`](config/config.example.yaml) for the complete, commented reference and
[`config/config.demo.yaml`](config/config.demo.yaml) for a runnable example.

**Environment substitution.** Any string may contain `${VAR}` or `${VAR:-default}`; booleans and numbers can
come from the environment too (`enabled: ${AUTH_ENABLED:-true}`). `sql:` values are never expanded.
Quote values that contain a colon, e.g. `"sqlite::memory:"`.

### Authentication

```yaml
auth:
  enabled: true        # false = ALL authorizers off; everyone is an anonymous administrator
  ldap:  { enabled: true, url: ldaps://ldap.example.org, bind_dn: ..., bind_password: ${LDAP_BIND_PASSWORD}, base_dn: ... }
  oidc:  { enabled: true, issuer_url: https://idp.example.org/realms/main, client_id: ..., client_secret: ${OIDC_CLIENT_SECRET} }
```

The OIDC redirect URI to register at the identity provider is `<server.public_url>/auth/callback`.
With `auth.enabled: true` at least one provider must be enabled, otherwise startup fails.

### Permissions

```yaml
access:
  default_roles: []                      # every authenticated user
  roles:
    admin:   ["*"]
    analyst: ["scripts.sales.*"]
  group_roles:                           # LDAP CN, LDAP DN or OIDC group name (case-insensitive)
    analysts: [analyst]
    "cn=admins,ou=groups,dc=example,dc=org": [admin]
  user_roles:
    alice: [admin]

scripts:
  - name: sales_by_region
    permissions: [scripts.sales.read]    # caller needs ALL listed permissions; empty = any signed-in user
```

Scripts a user may not run are hidden from `/` by default. "Show all scripts" (`/?all=1`) lists them as **locked**, with their description and the permissions they require, and a locked script cannot be opened (it returns `403`). The API does the same with `GET /api/scripts?all=1`, where each script carries `accessible` and `required_permissions`.

#### External grants service

Instead of the maps above, roles and permissions can come from a service you run. The portal checks it on each request and keeps the answers in a local cache:

```yaml
access:
  external:
    url: "https://permissions.internal/v1/permissions"   # http:// or https://
    timeout_secs: 3
    cache_ttl_secs: 300        # fresh answers are reused for this long
    bearer_token: "${PERMISSIONS_TOKEN}"                 # optional
    tls:                       # optional; otherwise the shared top-level tls block applies to https://
      ca_file: /etc/portal-tls/ca.crt
```

- Request: `POST {url}` with JSON `{"subject", "display_name", "email", "groups"}`.
- Response: `200` with `{"roles": [...], "permissions": [...]}`. `404` means the identity holds nothing.
- Anything else, or no answer, is an error. A user whose grants cannot be loaded is denied everything and the failure is logged. An answer from the cache can be used during an outage only while it is younger than twice `cache_ttl_secs`, so revoked access stops working within that window.
- The same TLS options as the database and LDAP connections apply: a CA, a client certificate for mutual TLS, or `insecure` for development.

### Scripts, forms and charts

```yaml
scripts:
  - name: top_products              # URL: /scripts/top_products  ([a-z0-9_-])
    title: Top products
    description: Best sellers, optionally per region.
    permissions: [scripts.sales.read]
    max_rows: 50                    # default 1000; extra rows are cut and flagged
    timeout_secs: 10                # default 30
    sql: |                          # or: sql_file: scripts/top_products.sql
      SELECT product, SUM(amount) AS revenue
      FROM sales
      WHERE (:region = '' OR region = :region) AND amount >= COALESCE(:min_amount, 0)
      GROUP BY product ORDER BY revenue DESC LIMIT :limit
    params:                         # each :name used in the SQL must be declared here
      - { name: region, type: select, default: "", options: [{ value: "", label: All }, EU, US] }
      - { name: min_amount, type: number, min: 0 }
      - { name: limit, type: integer, min: 1, max: 20, default: 5, required: true }
    charts:
      - { title: Revenue, type: pie, x: product, y: [revenue] }   # x = labels, y = value columns
    table: true                     # set false to show charts only
```

Parameter options: `label`, `required`, `default`, `help`, `min`/`max` (numbers), `max_length`/`pattern` (text),
`options` (select). Scripts whose parameters all have defaults run on page load; others run on form submit.
Parameters travel in the query string, so results are bookmarkable and shareable.

### SQL safety and database support

- `database.read_only_scripts: true` (default) accepts only a single `SELECT`/`WITH` statement without write
  keywords. It is a safety net — **also use a read-only database account**.
- Supported URLs: `sqlite:` and `postgres://` (via sqlx's `Any` driver), and `clickhouse://` /
  `clickhouses://` (HTTP interface, port 8123 / 8443 by default), e.g.
  `clickhouse://reader:secret@ch.internal:8123/analytics`.
- **Migrations are off by default.** Set `database.migrations.enabled: true` to apply the `*.sql` files of
  `database.migrations.dir` (default `migrations/sqlite`) in file-name order at startup, before `init_sql_files`.
  Nothing is applied implicitly, and a missing directory is a startup error. The demo configuration enables them
  explicitly to load its sample data.
- Each database's SQL rules live in its own crate under `datasources/`: `postgres-dialect`, `clickhouse-dialect`
  and `sqlite-dialect`, built on the `sql-executor` contract (`Dialect` trait, `Cell`, `Statement`). They do no I/O
  and can be used as dependencies. Adding a database means a new dialect crate plus a `QueryExecutor` adapter
  in `sql-portal/adapters/src/sql/`.
- ClickHouse: `:name` placeholders become typed server parameters (`{name:Int64}`, `{name:String}`, …), so
  values are never spliced into the SQL. Scripts run with `readonly=2` on the server, and each query gets a
  `max_execution_time` equal to the script timeout. Use a read-only ClickHouse user as well.
- **Type support follows sqlx `Any`**: integers, floats, booleans, text and blobs. Cast other types in SQL
  (`CAST(created_at AS TEXT)`, `price::float8`, SQLite `BOOLEAN` columns → `CAST(flag AS INTEGER)`).
- Postgres: pass typed values with casts where inference needs it, e.g. `CAST(:from_date AS date)`.
- **TLS for every outbound connection** (database, LDAP, OIDC). Set one shared `tls` block at the top level:
  ```yaml
  tls:
    ca_file: /etc/portal-tls/ca.crt     # CA, or the server's own self-signed certificate
    cert_file: /etc/portal-tls/tls.crt  # optional client certificate (mutual TLS)
    key_file: /etc/portal-tls/tls.key   # its private key
    insecure: false                     # true skips verification; development only
  ```
  A connection that uses TLS (`postgres://`, `clickhouses://`, `ldaps://` or `start_tls: true`, and OIDC) takes this block.
  Plain connections ignore it. To give one connection its own settings, add a `tls` block under it
  (`database.tls`, `auth.ldap.tls`, `auth.oidc.tls`); that block replaces the shared one.
- **Kubernetes:** mount a `kubernetes.io/tls` secret (`tls.crt`, `tls.key`) plus the CA (for example cert-manager's
  `ca.crt`) into the pod, and point the three paths at it. Relative paths resolve against the config file.
- `ca_file` verifies the chain and the host name. A self-signed server certificate must not have `CA:TRUE` (rustls
  rejects it as a leaf); issue it with `basicConstraints=CA:FALSE`.
- `cert_file` needs `key_file`, and needs `ca_file` (or `insecure`) so the server is still verified.
- Live tests (ignored by default): `cargo test -p sql-portal-integration-tests --test tls -- --ignored`, with `TLS_PG_URL`, `TLS_CH_URL`, `TLS_LDAP_URL`,
  `TLS_CA_FILE`, `TLS_PG_MTLS_URL`, `TLS_CLIENT_CERT`, `TLS_CLIENT_KEY` set for the servers you have.

## Try it locally: the showcase

The demo configuration enables the SQLite migrations explicitly (`database.migrations`), so the sample schema and data are created at startup in memory. Nothing else is needed:

```sh
make run          # cargo run -- --config config/config.demo.yaml
# open http://localhost:8080
```

The `Showcase` scripts cover every way the page presents selected data:

| Script | Presentation |
|---|---|
| Showcase 1 · table only | the default: sortable, filterable table, no charts |
| Showcase 2 · bar chart, two series | bar chart with two series, table underneath |
| Showcase 3 · line chart with gaps | line chart; NULL values break the line |
| Showcase 4 · pie chart | pie chart; a zero value has no slice |
| Showcase 5 · doughnut chart, table hidden | `table: false` |
| Showcase 6 · scatter plot, table hidden | numeric points, no table |
| Showcase 7 · several charts and a table | three charts of different kinds over one result |
| Showcase 8 · truncated result | `max_rows`: the badge shows the cap |
| Showcase 9 · no rows | empty states for charts and table |
| Showcase 10 · chart error next to a valid chart | a broken chart is reported, the other still draws |
| Showcase 11 · every parameter kind | text, integer, number, boolean, date, select; defaults, limits, help, validation |
| Showcase 12 · column types in a table | NULLs, 0/1 booleans, decimals, dates, markup-like and non-ASCII text |
| Showcase 13 · wide table | twelve columns; the header stays in place while scrolling |
| Showcase 14 · filters with a bar and a pie | select, date range and number filters feed two charts |
| Showcase 15 · time series with filters | region and date filters; a line over the months that have sales |
| Showcase 16 · top N with a switch | integer limit and a boolean switch; bar and doughnut |
| Showcase 17 · scatter with filters | segment and minimum-spend filters; scatter with a paged table |
| Showcase 18 · search with paging | free-text search, ten rows per page, no charts |
| Showcase 19 · 20,000 rows, 100 per page | only the requested page is fetched; the pager covers 200 pages |

The `Gallery` scripts put several chart kinds over the same data, with filters that change the charts:

| Script | Charts | Filters |
|---|---|---|
| Gallery 1 · all five chart kinds | line, bar, pie, doughnut, scatter | year (select) |
| Gallery 2 · quarterly revenue | bar (four lines), line (two premium lines) | year (select) |
| Gallery 3 · brand share | pie, doughnut | brand text, minimum share (number) |
| Gallery 4 · sensor readings, paged | line (temperature and humidity), scatter | location, date range |
| Gallery 5 · survey by age band | bar, line, doughnut | segment (select), minimum age (integer) |
| Gallery 6 · daily traffic, paged | line, bar | date range, minimum visits, weekdays only (boolean) |

The data and the scripts are in `config/migrations/sqlite/` and `config/config.demo.yaml`. The tests in `sql-portal/integration-tests/tests/presentation.rs` check each mode.

### Sharing a view

"Share link" above the results copies a link to the page with the parameters in the form, the page number and the rows per page. Opening the link shows the same rows. Without JavaScript the button is a plain link to the same view.

### Grouping scripts

Set `group` on a script to list it under a heading on the start page. Groups are sorted by name, case-insensitively; scripts without a group are listed last under "Other scripts". The search box matches the group name too, and hides headings that have no matches. The JSON API returns the group of each script.

```yaml
- name: monthly_trend
  title: Monthly trend
  group: Sales
```

### Paging

A script with a table shows one page at a time. The page size is `page_size` (default 25, between 1 and 500). The count and the page are two queries, so a large result is never loaded in full:

```yaml
- name: big_report
  page_size: 100
  max_rows: 200        # charts are drawn from the first 200 rows of the result
  sql: SELECT ... ORDER BY ...
```

Above the table, a "Rows per page" choice (5, 10, 25, 50, 100, and the script's own page size) starts at page 1 when changed. It is kept in the URL as `_size`, so `?_size=10&_page=3` shows rows 21–30. Scripts whose table is hidden have no paging and no choice.

The pager below the table shows `Page N of M`, arrows on both sides of the page numbers, and a window of page numbers around the current page with gaps for distant ones. It keeps the form values in each link. It is not shown when the result fits on one page. `?_page=N` selects a page; out-of-range numbers are clamped. Put an `ORDER BY` in the script so that the pages are stable. "Export page as CSV" exports the rows on the current page. The JSON API is not paged and still returns up to `max_rows` rows.

## GitHub sign-in

Sign in with a GitHub account. GitHub is OAuth 2.0 rather than OpenID Connect, so the portal talks to the GitHub API directly. The user's organisations and teams become access groups: an organisation is `acme`, a team is `acme/data-admins`. Start from `config/config.github.example.yaml`.

**1. Create the OAuth app.** On GitHub open *Settings → Developer settings → OAuth Apps → New OAuth App* (for an organisation: *Organization settings → Developer settings*). Fill in:

| Field | Value |
|---|---|
| Application name | any name, e.g. `Script portal` |
| Homepage URL | your `server.public_url`, e.g. `https://portal.example.org` |
| Authorization callback URL | `<public_url>/login/github/callback`, e.g. `https://portal.example.org/login/github/callback` |

Leave *Enable Device Flow* off. Click *Register application*.

**2. Copy the credentials.** The page shows the *Client ID*. Click *Generate a new client secret* and copy it now; GitHub shows it only once.

**3. Give the portal the credentials.** Keep the secret out of the files:

```sh
export GITHUB_CLIENT_ID=Iv1.xxxxxxxxxxxxxxxx
export GITHUB_CLIENT_SECRET=xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx
export PUBLIC_URL=https://portal.example.org
SCRIPT_PORTAL_CONFIG=config/config.github.example.yaml cargo run
```

**4. Let organisation members use the app.** Organisations that restrict OAuth apps must approve the app once: *Organization settings → Third-party access* → approve *Script portal*. Without this, members of that organisation are refused.

**5. Choose who may sign in and what they can do.**

- `allowed_orgs: ["acme"]` lets only members of `acme` in. Remove the line to accept any GitHub account.
- `group_roles` maps organisations (`acme`) and teams (`acme/data-admins`) to roles, as for every other sign-in method.
- `user_roles` maps a GitHub login to roles, e.g. `alice: [admin]`.

The scopes requested are `read:user`, `user:email` and `read:org`. `read:org` is needed for the organisations and teams.

**6. Check it.** Open `/login`, choose *Sign in with GitHub*, and approve the app. Your name appears at the top of the page, and the scripts your groups allow are listed.

**Notes**

- The user's identity is the GitHub login. If someone renames their account, their `user_roles` entry must be renamed too.
- Sign-in is refused if the state returned by GitHub does not match the one the portal started, if GitHub rejects the code, or if `allowed_orgs` is set and the user is in none of them.
- For GitHub Enterprise Server set `web_url` and `api_url` in `auth.github`.
- The callback must be reachable at exactly the URL registered in the OAuth app; a mismatch makes GitHub refuse the sign-in.

## HTTP endpoints

| Path | Purpose |
|---|---|
| `GET /` | List of scripts available to the signed-in user |
| `GET /scripts/{name}` | Parameter form and results (`?_run=1&param=value`) |
| `GET /login`, `POST /login` | LDAP sign-in form |
| `GET /login/oidc`, `GET /auth/callback` | OIDC sign-in |
| `POST /logout` | Sign out |
| `GET /api/scripts`, `GET /api/scripts/{name}?param=value` | JSON API (401 JSON when not signed in) |
| `GET /healthz` | Liveness: the process is up |
| `GET /readyz` | Readiness: `200`/`503` with per-check JSON (database is critical; OIDC discovery is reported but non-critical) |
| `GET /metrics` | Prometheus exposition (path configurable, can be disabled) |

## Metrics

| Metric | Type | Labels |
|---|---|---|
| `http_requests_total` | counter | `method`, `path` (route template), `status` |
| `http_request_duration_seconds` | histogram | `method`, `path`, `status` |
| `http_requests_in_flight` | gauge | |
| `auth_attempts_total` | counter | `provider`, `outcome` |
| `script_runs_total` | counter | `script`, `outcome` |
| `script_run_duration_seconds` | histogram | `script` |
| `script_rows_returned_total` | counter | `script` |
| `build_info`, `auth_enabled` | gauge | `version` |

Logs use `tracing` (`log_format: json` for log shippers); every request gets an `x-request-id` that appears in its span.

## Security notes

- Passwords are only ever sent to your directory (LDAP bind as the user); empty passwords are rejected up front
  (many servers treat them as anonymous binds). LDAP filter input is escaped. Use `ldaps://` or `start_tls`.
- Sessions use an HttpOnly, `SameSite=Lax` cookie (set `server.session.secure: true` behind HTTPS), a rotated id on login,
  and CSRF tokens on login/logout. OIDC uses PKCE, `state` and `nonce`; the pending state is single-use.
- Responses carry a strict CSP (scripts only from `self` and the configured chart library origin), `X-Frame-Options: DENY`, etc.
  Set `ui.chartjs_url` to a self-hosted file (e.g. `/static/chart.js` via your proxy) to avoid the CDN.
- **Not included** (put these in front, or extend): login rate limiting/lockout, TLS termination, a persistent/shared session store.
  Sessions are held **in memory**: they are lost on restart and not shared between replicas — run one replica or use sticky sessions,
  or swap `MemoryStore` in `sql-portal/api/src/router.rs` for a persistent `tower-sessions` store.
- Metrics and health endpoints are unauthenticated; restrict them at the network level if needed.

## Development

```bash
make lint         # rustfmt --check + clippy -D warnings
make test         # unit + in-process integration tests; no Docker needed
make test-docker  # everything above + PostgreSQL, ClickHouse and the portal image (builds it first)
make test-e2e     # only the end-to-end checks of the portal image
make coverage     # needs cargo-llvm-cov
```

**Tests.** Unit tests sit next to the code in each crate. Integration tests live in
`sql-portal/integration-tests/tests/`:

| file | runs against | needs |
|---|---|---|
| `http_api.rs`, `presentation.rs`, `github_login.rs`, `metrics.rs`, `grants.rs`, `bootstrap.rs`, `sqlite_and_access.rs` | the real router, in process, fake identity providers, SQLite in memory | nothing |
| `architecture.rs` | the Cargo dependency graph (layer rules) | nothing |
| `postgres.rs`, `clickhouse.rs` | PostgreSQL 16 / ClickHouse 24.8 containers | `--features docker` |
| `portal_image.rs` | the portal Docker image (`SCRIPT_PORTAL_IMAGE`, default `script-portal:test`), alone and with PostgreSQL on a shared network | `--features docker` + the image |
| `tls.rs` | TLS-enabled servers you provide (env vars) | `-- --ignored` |

Container tests use [testcontainers](https://docs.rs/testcontainers) and work with Docker or Podman
(`DOCKER_HOST` is honoured). Every container is stopped and removed when its test ends, including on
failure. Database containers are shared by the tests of one binary and removed after the last one.
The portal image is never pulled, so build it first (`make image-test`). If a run is killed before
cleanup, `make test-docker-clean` removes leftovers (they carry the label `org.sql-portal.integration-test`).

The LDAP and OIDC network paths (`ldap.rs`, `oidc.rs`) are covered by unit tests of their pure logic
(filter escaping, claim/entry mapping, error handling) but **not** by tests against a live directory or IdP.
Verify them against your environment (a `lldap` or Keycloak container works well) before going live.

### CI and releases

- `.github/workflows/ci.yml` runs on every pull request: rustfmt, clippy (`-D warnings`), tests, coverage
  (`cargo-llvm-cov`, fails below `COVERAGE_MIN`, default 70 %), the container tests against a freshly built image
  (plus a check of the image's `HEALTHCHECK` under Docker), and `cargo audit`.
- `.github/workflows/release.yml` runs on tags like `v1.2.3`:
  it re-runs the checks (the tag must match the version in `sql-portal/deployment/Cargo.toml`), builds binaries (Linux x86_64/arm64, macOS arm64, Windows x86_64) and attaches them with
  checksums to the GitHub release, and pushes a multi-arch image to Docker Hub.
  Configure the secrets `DOCKERHUB_USERNAME` and `DOCKERHUB_TOKEN` (and optionally the variable `DOCKERHUB_REPOSITORY`).

```bash
# cut a release
# 1. bump version in Cargo.toml, commit
git tag v0.2.0 && git push origin v0.2.0
```

## License

MIT
