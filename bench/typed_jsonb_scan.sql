-- One pooled request, typed projection, and the complete JSONB source row.
SELECT id, display_name, created_at, attrs
FROM imported.list_items
LIMIT 1;
