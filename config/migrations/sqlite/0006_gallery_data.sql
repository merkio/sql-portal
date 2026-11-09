-- Deterministic data for the gallery_* scripts.

WITH RECURSIVE q(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM q WHERE i < 7),
     l(j, name) AS (VALUES (0, 'Core'), (1, 'Plus'), (2, 'Pro'), (3, 'Enterprise'))
INSERT INTO quarterly_sales (quarter, product_line, units, revenue)
SELECT (2023 + i / 4) || '-Q' || (i % 4 + 1),
       name,
       40 + (i * 17 + j * 23) % 90,
       round((40 + (i * 17 + j * 23) % 90) * (j + 1) * 120.0 + i * 900, 2)
FROM q CROSS JOIN l;

INSERT INTO brand_share (brand, share, units) VALUES
    ('Acme', 31.5, 420),
    ('Globex', 22.0, 310),
    ('Initech', 14.25, 205),
    ('Umbrella', 12.5, 180),
    ('Hooli', 11.0, 150),
    ('Others', 8.75, 120);

WITH RECURSIVE h(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM h WHERE i < 335),
     loc(name, offset) AS (VALUES ('Warehouse', 0.0), ('Office', 3.5))
INSERT INTO sensor_readings (ts, location, temperature, humidity)
SELECT strftime('%Y-%m-%d %H:00', datetime('2024-06-01 00:00', '+' || i || ' hours')),
       name,
       round(18.0 + offset + ((i % 24) BETWEEN 12 AND 17) * 2.5 + ((i * 7) % 13) * 0.3, 1),
       round(60.0 - offset * 2 - ((i % 24) BETWEEN 12 AND 17) * 8 + ((i * 5) % 11) * 0.6, 1)
FROM h CROSS JOIN loc;

WITH RECURSIVE s(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM s WHERE i < 200)
INSERT INTO survey_responses (id, age, segment, satisfaction, spend)
SELECT i,
       18 + (i * 29) % 52,
       CASE i % 3 WHEN 0 THEN 'consumer' WHEN 1 THEN 'business' ELSE 'partner' END,
       1 + (i * 7) % 10,
       round(20 + (i * 13) % 180 + ((i * 7) % 10) * 3.5, 2)
FROM s;

WITH RECURSIVE d(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM d WHERE i < 59)
INSERT INTO web_traffic (day, direct, search, social)
SELECT date('2024-05-01', '+' || i || ' days'),
       800 + (i * 37) % 240 + (i % 7 IN (5, 6)) * 300,
       1200 + (i * 53) % 400 - (i % 7 IN (5, 6)) * 200,
       300 + (i * 29) % 180 + (i % 7 IN (0, 6)) * 150
FROM d;
