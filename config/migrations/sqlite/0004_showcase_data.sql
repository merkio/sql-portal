-- Deterministic sample data for the showcase scripts.

WITH RECURSIVE m(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM m WHERE i < 23)
INSERT INTO showcase_monthly (month, orders, revenue, returns)
SELECT
    strftime('%Y-%m', date('2023-01-01', '+' || i || ' months')),
    CASE WHEN i IN (5, 6) THEN NULL ELSE 40 + (i * 7) % 23 END,
    CASE WHEN i = 11 THEN NULL ELSE round(4000 + i * 180 + (i % 4) * 350.5, 2) END,
    CASE WHEN i % 5 = 0 THEN 0 ELSE (i * 3) % 9 END
FROM m;

INSERT INTO showcase_categories (category, units) VALUES
    ('Electronics', 320),
    ('Books', 150),
    ('Garden', 0),
    ('Toys', 210),
    ('Clothing', 280);

WITH RECURSIVE p(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM p WHERE i < 59)
INSERT INTO showcase_points (id, advertising, sales, segment)
SELECT
    i + 1,
    10 + (i * 37) % 90 + 0.5 * (i % 3),
    round((10 + (i * 37) % 90) * 1.8 + (i * 13) % 17 - 8, 2),
    CASE i % 3 WHEN 0 THEN 'consumer' WHEN 1 THEN 'business' ELSE 'partner' END
FROM p;

INSERT INTO showcase_items (id, label, note, active, price, quantity, added_on, description, markup, city) VALUES
    (1, 'Widget', 'Standard', 1, 9.99, 12, '2024-01-05',
     'A basic widget used in most orders.', '<b>bold</b> & "quoted"', 'Kraków'),
    (2, 'Gadget', NULL, 0, NULL, 0, '2024-02-14',
     'Discontinued item without a price yet.', '<script>alert(1)</script>', 'Zürich'),
    (3, 'Doohickey', 'Limited', 1, 24.5, 3, '2024-03-01',
     'A long description that keeps going so the cell has to wrap or scroll gracefully inside the table layout of the showcase page.',
     'a & b', 'São Paulo'),
    (4, 'Gizmo', NULL, 1, 4.25, 150, '2024-04-20', 'Bulk item.', 'ok', '東京'),
    (5, 'Thingamajig', '', 1, 0, 1, '2024-05-11', 'Free sample.', 'plain', 'Oslo'),
    (6, 'Whatsit', 'Refurbished', 1, 199.0, 2, '2024-06-30', 'Expensive.', 'plain', 'Montréal'),
    (7, 'Doodad', NULL, 0, 15.0, 40, '2024-07-07', 'Inactive item.', 'plain', 'Lagos'),
    (8, 'Contraption', 'Beta', 1, 1234.5678, 1, '2024-08-08', 'Large price with decimals.', 'plain', 'Reykjavík');
