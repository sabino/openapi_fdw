-- One pooled localhost HTTP request plus JSON decoding and row projection.
SELECT id FROM imported.list_items LIMIT 1;
