#!/bin/sh
# One-time, offline consolidation for the local symbiont-d PCP Store.
#
# It retains only raw conversation records, moves them into the `symbiont-d`
# scope, and removes the legacy project/user-context material together with all
# links, provenance, indexes, and audit events that could still disclose it.
set -eu

SOURCE_SCOPE='conversation:symbiont-d-main'
TARGET_SCOPE='symbiont-d'

usage() {
  echo "usage: sh scripts/consolidate-symbiont-d-scope.sh --database <context.sqlite3> --apply" >&2
  exit 2
}

DATABASE=''
APPLY='false'
while [ "$#" -gt 0 ]; do
  case "$1" in
    --database) DATABASE="${2:-}"; shift 2 ;;
    --apply) APPLY='true'; shift ;;
    *) usage ;;
  esac
done

[ -n "$DATABASE" ] || usage
[ "$APPLY" = 'true' ] || usage
[ -f "$DATABASE" ] || { echo "Store is not a regular file: $DATABASE" >&2; exit 1; }
command -v sqlite3 >/dev/null || { echo 'sqlite3 is required' >&2; exit 1; }
command -v lsof >/dev/null || { echo 'lsof is required to prove the Store is offline' >&2; exit 1; }

# SQLite permits concurrent readers, but a process that still has this Store
# open can later write an old scope back. Refuse the migration rather than
# racing Runtime or Console.
if lsof "$DATABASE" >/dev/null 2>&1; then
  echo "Refusing to migrate an open Store: stop PCP Runtime/Console first." >&2
  lsof "$DATABASE" >&2 || true
  exit 1
fi

for TABLE in \
  pcp_scopes pcp_pages pcp_revisions pcp_relations pcp_relation_retractions \
  pcp_provenance_inputs pcp_summaries pcp_page_summary_heads \
  pcp_summary_idempotency pcp_summary_assessments pcp_validity_assessments \
  pcp_validity_heads pcp_validity_idempotency pcp_idempotency \
  pcp_revision_retention_leases pcp_revision_collections pcp_page_packs \
  pcp_access_log pcp_revision_fts pcp_summary_fts
do
  FOUND="$(sqlite3 "$DATABASE" "SELECT COUNT(*) FROM sqlite_master WHERE type IN ('table', 'view') AND name = '$TABLE';")"
  [ "$FOUND" = '1' ] || { echo "Store misses required table: $TABLE" >&2; exit 1; }
done

SCOPE_COUNT="$(sqlite3 "$DATABASE" 'SELECT COUNT(*) FROM pcp_scopes;')"
SOURCE_COUNT="$(sqlite3 "$DATABASE" "SELECT COUNT(*) FROM pcp_scopes WHERE namespace = '$SOURCE_SCOPE';")"
TARGET_COUNT="$(sqlite3 "$DATABASE" "SELECT COUNT(*) FROM pcp_scopes WHERE namespace = '$TARGET_SCOPE';")"
[ "$SCOPE_COUNT" = '3' ] || { echo "Refusing Store with $SCOPE_COUNT scopes; expected exactly 3 for this one-time migration." >&2; exit 1; }
[ "$SOURCE_COUNT" = '1' ] || { echo "Missing source scope: $SOURCE_SCOPE" >&2; exit 1; }
[ "$TARGET_COUNT" = '0' ] || { echo "Target scope already exists: $TARGET_SCOPE" >&2; exit 1; }
[ "$(sqlite3 "$DATABASE" 'PRAGMA quick_check;')" = 'ok' ] || { echo 'Source Store failed SQLite quick_check.' >&2; exit 1; }

MIGRATION_TIMESTAMP="$(date -u '+%Y%m%dT%H%M%SZ')"
BACKUP="${DATABASE%.sqlite3}.before-symbiont-d-scope-consolidation.${MIGRATION_TIMESTAMP}.sqlite3"
SQL_FILE="$(mktemp -t pcp-symbiont-d-scope.XXXXXX)"
trap 'rm -f "$SQL_FILE"' EXIT HUP INT TERM

if [ "$(sqlite3 "$DATABASE" "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'pcp_query_audit';")" = '1' ]; then
  QUERY_AUDIT_SQL='DELETE FROM pcp_query_audit;'
else
  QUERY_AUDIT_SQL=''
fi

echo "Creating verified backup: $BACKUP"
sqlite3 "$DATABASE" ".backup '$BACKUP'"
[ "$(sqlite3 "$BACKUP" 'PRAGMA quick_check;')" = 'ok' ] || {
  echo 'Backup failed SQLite quick_check; original Store was not changed.' >&2
  exit 1
}
chmod 600 "$BACKUP"

cat >"$SQL_FILE" <<SQL
PRAGMA foreign_keys = OFF;
BEGIN IMMEDIATE;

-- Keep the raw conversation scope, except Pages whose provenance relies on a
-- legacy scope. Keeping those records would leave a false or dangling lineage.
CREATE TEMP TABLE drop_pages(page_id TEXT PRIMARY KEY);
INSERT INTO drop_pages(page_id)
SELECT page_id
FROM pcp_pages
WHERE namespace <> '$SOURCE_SCOPE';
INSERT OR IGNORE INTO drop_pages(page_id)
SELECT DISTINCT derived.page_id
FROM pcp_provenance_inputs AS provenance
JOIN pcp_revisions AS derived ON derived.revision_id = provenance.derived_revision_id
JOIN pcp_revisions AS input ON input.revision_id = provenance.input_revision_id
WHERE derived.namespace = '$SOURCE_SCOPE'
  AND input.namespace <> '$SOURCE_SCOPE';

CREATE TEMP TABLE drop_revisions(revision_id TEXT PRIMARY KEY);
INSERT INTO drop_revisions(revision_id)
SELECT revision_id FROM pcp_revisions
WHERE page_id IN (SELECT page_id FROM drop_pages);

CREATE TEMP TABLE drop_relations(relation_id TEXT PRIMARY KEY);
INSERT INTO drop_relations(relation_id)
SELECT relation_id
FROM pcp_relations
WHERE from_page_id IN (SELECT page_id FROM drop_pages)
   OR to_page_id IN (SELECT page_id FROM drop_pages)
   OR EXISTS (
     SELECT 1
     FROM json_each(pcp_relations.basis_revision_ids_json) AS basis
     JOIN pcp_revisions AS revision ON revision.revision_id = basis.value
     WHERE revision.namespace <> '$SOURCE_SCOPE'
   );

DELETE FROM pcp_relation_retractions
WHERE relation_id IN (SELECT relation_id FROM drop_relations);
DELETE FROM pcp_relations
WHERE relation_id IN (SELECT relation_id FROM drop_relations);

DELETE FROM pcp_provenance_inputs
WHERE derived_revision_id IN (SELECT revision_id FROM drop_revisions)
   OR input_revision_id IN (SELECT revision_id FROM drop_revisions);
DELETE FROM pcp_summary_fts
WHERE summary_revision_id IN (SELECT revision_id FROM drop_revisions)
   OR target_revision_id IN (SELECT revision_id FROM drop_revisions);
DELETE FROM pcp_summary_idempotency;
DELETE FROM pcp_summary_assessments
WHERE target_revision_id IN (SELECT revision_id FROM drop_revisions);
DELETE FROM pcp_summaries
WHERE summary_revision_id IN (SELECT revision_id FROM drop_revisions)
   OR target_revision_id IN (SELECT revision_id FROM drop_revisions);
DELETE FROM pcp_page_summary_heads
WHERE target_page_id IN (SELECT page_id FROM drop_pages)
   OR summary_page_id IN (SELECT page_id FROM drop_pages);

DELETE FROM pcp_validity_idempotency;
DELETE FROM pcp_validity_assessments
WHERE assessment_revision_id IN (SELECT revision_id FROM drop_revisions)
   OR target_revision_id IN (SELECT revision_id FROM drop_revisions);
DELETE FROM pcp_validity_heads
WHERE target_page_id IN (SELECT page_id FROM drop_pages)
   OR assessment_page_id IN (SELECT page_id FROM drop_pages);

DELETE FROM pcp_revision_retention_leases
WHERE page_id IN (SELECT page_id FROM drop_pages)
   OR revision_id IN (SELECT revision_id FROM drop_revisions);
DELETE FROM pcp_revision_collections
WHERE page_id IN (SELECT page_id FROM drop_pages)
   OR revision_id IN (SELECT revision_id FROM drop_revisions);
DELETE FROM pcp_page_packs
WHERE source_page_id IN (SELECT page_id FROM drop_pages)
   OR packed_page_id IN (SELECT page_id FROM drop_pages)
   OR source_revision_id IN (SELECT revision_id FROM drop_revisions)
   OR packed_revision_id IN (SELECT revision_id FROM drop_revisions);
DELETE FROM pcp_revision_fts
WHERE page_id IN (SELECT page_id FROM drop_pages)
   OR revision_id IN (SELECT revision_id FROM drop_revisions);

-- Idempotency and audit entries can contain old Page IDs or old scope grants.
-- Clear them rather than retaining stale operational authority/history.
DELETE FROM pcp_idempotency;
DELETE FROM pcp_access_log;
$QUERY_AUDIT_SQL

DELETE FROM pcp_revisions
WHERE revision_id IN (SELECT revision_id FROM drop_revisions);
DELETE FROM pcp_pages
WHERE page_id IN (SELECT page_id FROM drop_pages);

INSERT INTO pcp_scopes (
  namespace, display_name, description, parent_namespace, created_at, updated_at
)
SELECT '$TARGET_SCOPE', 'symbiont-d',
       'Raw conversation records owned by symbiont-d.', NULL,
       created_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
FROM pcp_scopes
WHERE namespace = '$SOURCE_SCOPE';

UPDATE pcp_pages
SET namespace = '$TARGET_SCOPE'
WHERE namespace = '$SOURCE_SCOPE';
UPDATE pcp_revisions
SET namespace = '$TARGET_SCOPE'
WHERE namespace = '$SOURCE_SCOPE';
UPDATE pcp_revision_fts
SET namespace = '$TARGET_SCOPE'
WHERE namespace = '$SOURCE_SCOPE';
UPDATE pcp_revision_retention_leases
SET namespace = '$TARGET_SCOPE'
WHERE namespace = '$SOURCE_SCOPE';
UPDATE pcp_revision_collections
SET namespace = '$TARGET_SCOPE'
WHERE namespace = '$SOURCE_SCOPE';
UPDATE pcp_page_packs
SET namespace = '$TARGET_SCOPE'
WHERE namespace = '$SOURCE_SCOPE';

DELETE FROM pcp_scopes
WHERE namespace <> '$TARGET_SCOPE';
COMMIT;
PRAGMA foreign_keys = ON;
PRAGMA foreign_key_check;
SQL

sqlite3 -bail "$DATABASE" <"$SQL_FILE"
sqlite3 "$DATABASE" 'VACUUM;'
[ "$(sqlite3 "$DATABASE" 'PRAGMA quick_check;')" = 'ok' ] || {
  echo "Migrated Store failed SQLite quick_check. Restore from $BACKUP before starting Runtime." >&2
  exit 1
}
[ "$(sqlite3 "$DATABASE" 'PRAGMA foreign_key_check;' | wc -l | tr -d ' ')" = '0' ] || {
  echo "Migrated Store failed foreign_key_check. Restore from $BACKUP before starting Runtime." >&2
  exit 1
}
[ "$(sqlite3 "$DATABASE" 'SELECT COUNT(*) FROM pcp_scopes;')" = '1' ] || {
  echo "Migration did not reduce Store to one scope. Restore from $BACKUP." >&2
  exit 1
}
[ "$(sqlite3 "$DATABASE" "SELECT COUNT(*) FROM pcp_scopes WHERE namespace = '$TARGET_SCOPE';")" = '1' ] || {
  echo "Target scope is missing after migration. Restore from $BACKUP." >&2
  exit 1
}
[ "$(sqlite3 "$DATABASE" "SELECT COUNT(*) FROM pcp_pages WHERE namespace <> '$TARGET_SCOPE';")" = '0' ] || {
  echo "Non-target Pages remain after migration. Restore from $BACKUP." >&2
  exit 1
}
[ "$(sqlite3 "$DATABASE" "SELECT COUNT(*) FROM pcp_revisions WHERE namespace <> '$TARGET_SCOPE';")" = '0' ] || {
  echo "Non-target Revisions remain after migration. Restore from $BACKUP." >&2
  exit 1
}

echo "Consolidated Store into scope: $TARGET_SCOPE"
echo "Backup retained at: $BACKUP"
echo "Configure Runtime, maintenance, and each enrolled client to grant only: $TARGET_SCOPE"
