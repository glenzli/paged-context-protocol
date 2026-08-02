use anyhow::{Context, Result};
use pcp_core::{
    Actor, ActorType, LifecycleStatus, PagePayload, PageRevision, ProvenanceEvent, Relation,
    SourceRef,
};
use rusqlite::Row;

pub(crate) fn revision_from_row(
    row: &Row<'_>,
    include_payload: bool,
    include_facets: bool,
    include_sources: bool,
    include_provenance: bool,
) -> Result<PageRevision> {
    let actor_type_text: String = row.get(10)?;
    let lifecycle_text: String = row.get(5)?;
    let payload_media_type: Option<String> = row.get(12)?;
    let payload_content: Option<String> = row.get(13)?;
    let source_refs_json: String = row.get(14)?;
    let facets_json: Option<String> = row.get(15)?;
    let provenance_json: String = row.get(16)?;

    Ok(PageRevision {
        page_id: row.get(0)?,
        revision_id: row.get(1)?,
        owner_id: row.get(2)?,
        namespace: row.get(3)?,
        visibility: row.get(4)?,
        lifecycle_status: LifecycleStatus::parse(&lifecycle_text)
            .with_context(|| format!("unknown lifecycle status {lifecycle_text}"))?,
        created_at: row.get(6)?,
        observed_at: row.get(7)?,
        valid_from: row.get(8)?,
        valid_to: row.get(9)?,
        created_by: Actor {
            actor_type: ActorType::parse(&actor_type_text)
                .with_context(|| format!("unknown actor type {actor_type_text}"))?,
            actor_id: row.get(11)?,
        },
        payload: if include_payload {
            payload_media_type
                .zip(payload_content)
                .map(|(media_type, content)| PagePayload {
                    media_type,
                    content,
                })
        } else {
            None
        },
        source_refs: if include_sources {
            serde_json::from_str::<Vec<SourceRef>>(&source_refs_json)
                .context("decode PCP source refs")?
        } else {
            Vec::new()
        },
        facets: if include_facets {
            facets_json
                .map(|value| serde_json::from_str(&value).context("decode PCP facets"))
                .transpose()?
        } else {
            None
        },
        provenance: if include_provenance {
            serde_json::from_str::<Vec<ProvenanceEvent>>(&provenance_json)
                .context("decode PCP provenance")?
        } else {
            Vec::new()
        },
    })
}

pub(crate) fn relation_from_row(row: &Row<'_>) -> Result<Relation> {
    let actor_type_text: String = row.get(4)?;
    Ok(Relation {
        relation_id: row.get(0)?,
        from_revision_id: row.get(1)?,
        relation_type: row.get(2)?,
        to_revision_id: row.get(3)?,
        created_by: Actor {
            actor_type: ActorType::parse(&actor_type_text)
                .with_context(|| format!("unknown actor type {actor_type_text}"))?,
            actor_id: row.get(5)?,
        },
        created_at: row.get(6)?,
    })
}

pub(crate) const REVISION_COLUMNS: &str = "
    r.page_id, r.revision_id, r.owner_id, r.namespace, r.visibility,
    r.lifecycle_status, r.created_at, r.observed_at, r.valid_from, r.valid_to,
    r.actor_type, r.actor_id, r.payload_media_type, r.payload_content,
    r.source_refs_json, r.facets_json, r.provenance_json
";
