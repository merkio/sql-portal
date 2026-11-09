-- Schema of the demo sales table.
CREATE TABLE sales (
    id        INTEGER PRIMARY KEY,
    region    TEXT    NOT NULL,
    product   TEXT    NOT NULL,
    sold_on   TEXT    NOT NULL,
    quantity  INTEGER NOT NULL,
    amount    REAL    NOT NULL
);
