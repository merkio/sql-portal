-- Gallery tables: data for the gallery_* scripts, one or more per chart kind.

-- Revenue and units per quarter and product line (8 quarters x 4 lines).
CREATE TABLE quarterly_sales (
    quarter      TEXT NOT NULL,        -- '2023-Q1'
    product_line TEXT NOT NULL,
    units        INTEGER NOT NULL,
    revenue      REAL NOT NULL,
    PRIMARY KEY (quarter, product_line)
);

-- Market share by brand.
CREATE TABLE brand_share (
    brand TEXT PRIMARY KEY,
    share REAL NOT NULL,               -- percent
    units INTEGER NOT NULL
);

-- Hourly sensor readings for two locations.
CREATE TABLE sensor_readings (
    ts          TEXT NOT NULL,         -- 'YYYY-MM-DD HH:00'
    location    TEXT NOT NULL,
    temperature REAL NOT NULL,
    humidity    REAL NOT NULL,
    PRIMARY KEY (ts, location)
);

-- Survey responses with an age, a segment and a satisfaction score.
CREATE TABLE survey_responses (
    id           INTEGER PRIMARY KEY,
    age          INTEGER NOT NULL,
    segment      TEXT NOT NULL,
    satisfaction INTEGER NOT NULL,     -- 1..10
    spend        REAL NOT NULL
);

-- Daily visits per channel.
CREATE TABLE web_traffic (
    day    TEXT PRIMARY KEY,           -- 'YYYY-MM-DD'
    direct INTEGER NOT NULL,
    search INTEGER NOT NULL,
    social INTEGER NOT NULL
);
