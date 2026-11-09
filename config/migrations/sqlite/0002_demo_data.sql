-- Deterministic demo sales: 180 days, three regions, four products.
WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < 179)
INSERT INTO sales (region, product, sold_on, quantity, amount)
SELECT
    CASE i % 3 WHEN 0 THEN 'EU' WHEN 1 THEN 'US' ELSE 'APAC' END,
    CASE i % 4 WHEN 0 THEN 'Widget' WHEN 1 THEN 'Gadget' WHEN 2 THEN 'Doohickey' ELSE 'Gizmo' END,
    date('2024-01-01', '+' || i || ' days'),
    1 + (i * 7) % 9,
    round((1 + (i * 7) % 9) * (10 + (i * 13) % 40) + (i % 5) * 0.25, 2)
FROM n;
