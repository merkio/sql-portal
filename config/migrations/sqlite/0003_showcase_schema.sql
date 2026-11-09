-- Showcase tables. Each exists to exercise one way of presenting results:
-- see the showcase_* scripts in config/config.demo.yaml.

-- Monthly series with gaps: NULL orders/revenue are drawn as breaks in lines.
CREATE TABLE showcase_monthly (
    month    TEXT PRIMARY KEY,      -- 'YYYY-MM'
    orders   INTEGER,               -- NULL = not reported
    revenue  REAL,                  -- NULL = not reported
    returns  INTEGER NOT NULL
);

-- Shares for pie and doughnut charts; one category is zero on purpose.
CREATE TABLE showcase_categories (
    category TEXT PRIMARY KEY,
    units    INTEGER NOT NULL
);

-- Numeric pairs for scatter charts, with a segment column for context.
CREATE TABLE showcase_points (
    id          INTEGER PRIMARY KEY,
    advertising REAL NOT NULL,
    sales       REAL NOT NULL,
    segment     TEXT NOT NULL
);

-- Mixed column types for the table: NULLs, booleans as 0/1, decimals, dates,
-- long text, markup-looking text and non-ASCII text.
CREATE TABLE showcase_items (
    id          INTEGER PRIMARY KEY,
    label       TEXT NOT NULL,
    note        TEXT,
    active      INTEGER NOT NULL,
    price       REAL,
    quantity    INTEGER NOT NULL,
    added_on    TEXT NOT NULL,
    description TEXT NOT NULL,
    markup      TEXT NOT NULL,
    city        TEXT NOT NULL
);
